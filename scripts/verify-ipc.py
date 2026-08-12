#!python3
"""部署后 IPC 健康验证：直连正在运行的 shurufa-algo 服务测请求往返延迟。

固定部署流程（scripts/update-all.ps1）的验收步骤：ToggleAscii 往返必须
亚毫秒级（p95 < 5ms）；若 500ms 内无响应说明服务端卡死（曾因 toggle_ascii
嵌套锁自死锁导致全线 2s 超时，见 tsf-must-never-block-ui-thread memory）。

依赖：python stdlib + ctypes（无第三方库）。
用法：python verify-ipc.py [--loops N]
退出码：0 = 健康；1 = 连接失败/超时/延迟异常。
"""
import ctypes, ctypes.wintypes as wt, json, sys, time

kernel32 = ctypes.windll.kernel32
GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
OPEN_EXISTING = 3
PIPE_READMODE_MESSAGE = 0x0002
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value

PIPE = r"\\.\pipe\shurufa-algo"


def open_pipe(timeout_ms=3000):
    if not kernel32.WaitNamedPipeW(PIPE, timeout_ms):
        return None
    h = kernel32.CreateFileW(
        PIPE, GENERIC_READ | GENERIC_WRITE, 0, None, OPEN_EXISTING, 0, None)
    if h == INVALID_HANDLE_VALUE:
        return None
    kernel32.SetNamedPipeHandleState(
        h, ctypes.byref(ctypes.c_ulong(PIPE_READMODE_MESSAGE)), None, None)
    return h


def write_frame(h, data: bytes) -> bool:
    buf = (ctypes.c_ubyte * len(data)).from_buffer_copy(data)
    written = ctypes.c_ulong(0)
    return bool(kernel32.WriteFile(h, buf, len(data), ctypes.byref(written), None))


def read_frame(h, timeout_ms=2000):
    deadline = time.perf_counter() + timeout_ms / 1000
    avail = ctypes.c_ulong(0)
    total = ctypes.c_ulong(0)
    while time.perf_counter() < deadline:
        ok = kernel32.PeekNamedPipe(
            h, None, 0, None, ctypes.byref(avail), ctypes.byref(total))
        if not ok:
            return None
        if avail.value > 0:
            n = avail.value
            buf = (ctypes.c_ubyte * n)()
            read = ctypes.c_ulong(0)
            if not kernel32.ReadFile(h, buf, n, ctypes.byref(read), None):
                return None
            return bytes(buf)[:read.value]
        time.sleep(0.002)
    return None


def frame(payload: str) -> bytes:
    b = payload.encode("utf-8")
    return len(b).to_bytes(4, "little") + b


def roundtrip(h, payload: str):
    t0 = time.perf_counter()
    if not write_frame(h, frame(payload)):
        return None, "write failed"
    resp = read_frame(h)
    dt = (time.perf_counter() - t0) * 1000
    if resp is None:
        return dt, "no response (timeout)"
    return dt, resp[4:].decode("utf-8", "replace")


def main():
    loops = 10
    if "--loops" in sys.argv:
        loops = int(sys.argv[sys.argv.index("--loops") + 1])
    h = open_pipe()
    if h is None:
        print("FAIL: 无法连接算法服务管道（服务未运行？）")
        sys.exit(1)
    dt, body = roundtrip(h, '"CreateSession"')
    if body == "no response (timeout)":
        print(f"FAIL: CreateSession 无响应（{dt:.0f}ms）——服务端卡死")
        kernel32.CloseHandle(h)
        sys.exit(1)
    # ToggleAscii 连续往返：每个请求都是独立锁路径，最能暴露死锁
    times = []
    for i in range(loops):
        dt, body = roundtrip(h, '"ToggleAscii"')
        if body == "no response (timeout)":
            print(f"FAIL: ToggleAscii #{i} 无响应（{dt:.0f}ms）——服务端卡死")
            kernel32.CloseHandle(h)
            sys.exit(1)
        times.append(dt)
    # 多连接并发：连接 1 保持打开时连接 2 必须独立服务
    h2 = open_pipe()
    if h2 is not None:
        dt2, body2 = roundtrip(h2, '"ToggleAscii"')
        if body2 == "no response (timeout)":
            print(f"FAIL: 连接2 ToggleAscii 无响应（{dt2:.0f}ms）——死锁拖垮并发连接")
            kernel32.CloseHandle(h2)
            kernel32.CloseHandle(h)
            sys.exit(1)
        times.append(dt2)
        kernel32.CloseHandle(h2)
    kernel32.CloseHandle(h)
    times.sort()
    p50, p95 = times[len(times) // 2], times[min(int(len(times) * 0.95), len(times) - 1)]
    ok = p95 < 5.0
    print(f"OK: ToggleAscii p50={p50:.2f}ms p95={p95:.2f}ms max={times[-1]:.2f}ms "
          f"({len(times)} 次往返)")
    sys.exit(0 if ok else 2)


if __name__ == "__main__":
    main()
