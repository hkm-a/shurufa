// 构建脚本：把 rime.dll 复制到测试可执行文件所在目录。
// Windows 下 rime.dll 由 ffi::loader 运行期显式加载（不静态链接），
// 因此这里只负责让测试进程在自身目录找到它。
use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest.parent().unwrap().parent().unwrap();
    let lib_dir = repo_root.join("third_party/librime/dist/lib");
    println!("cargo:rerun-if-changed={}", lib_dir.display());

    // OUT_DIR 形如 target/debug/build/<crate>-<hash>/out，
    // 向上回溯三层得到 target/debug，dll 需同时出现在该目录及 deps 下。
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
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
