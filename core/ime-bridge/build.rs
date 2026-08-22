// 构建脚本：
// 1. 用 bindgen 从仓内 rime_api.h 生成 librime C API 的 Rust 绑定；
// 2. 把 rime.dll 复制到测试可执行文件所在目录。
// Windows 下 rime.dll 由 ffi::loader 运行期显式加载（不静态链接），
// 因此这里只负责让测试进程在自身目录找到它。
use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest.parent().unwrap().parent().unwrap();
    let header = repo_root.join("third_party/librime/dist/include/rime_api.h");
    let lib_dir = repo_root.join("third_party/librime/dist/lib");
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-changed={}", lib_dir.display());

    // bindgen 生成 rime_bindings.rs
    // 本机刚装 LLVM 时 clang-sys 不一定能自动找到 libclang.dll，显式指定。
    if std::env::var_os("LIBCLANG_PATH").is_none() {
        let candidates = [
            "C:\\Program Files\\LLVM\\bin",
            "C:\\Program Files\\LLVM\\lib",
        ];
        let mut found = candidates
            .iter()
            .find(|dir| PathBuf::from(dir).join("libclang.dll").exists())
            .map(|s| s.to_string());
        if found.is_none() {
            // Python libclang wheel 也携带 libclang.dll，本机常用路径。
            if let Some(home) = std::env::var_os("USERPROFILE") {
                let py_native = PathBuf::from(&home)
                    .join("AppData/Roaming/Python/Python314/site-packages/clang/native");
                if py_native.join("libclang.dll").exists() {
                    found = Some(py_native.to_string_lossy().into_owned());
                }
            }
        }
        if let Some(dir) = found {
            std::env::set_var("LIBCLANG_PATH", dir);
        }
    }
    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy().into_owned())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen 生成 rime_api.h 绑定失败");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_dir.join("rime_bindings.rs"))
        .expect("写入 rime_bindings.rs 失败");

    // OUT_DIR 形如 target/debug/build/<crate>-<hash>/out，
    // 向上回溯三层得到 target/debug，dll 需同时出现在该目录及 deps 下。
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR 目录层级不符合预期")
        .to_path_buf();
    let dll = lib_dir.join("rime.dll");
    if dll.exists() {
        for target in [
            profile_dir.join("rime.dll"),
            profile_dir.join("deps/rime.dll"),
        ] {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if let Err(e) = std::fs::copy(&dll, &target) {
                // 目标被运行中的进程映射时复制失败；已有旧副本则容忍
                if !target.exists() {
                    panic!("复制 rime.dll 失败: {e}");
                }
                println!("cargo:warning=rime.dll 被占用未更新: {e}");
            }
        }
    }
}
