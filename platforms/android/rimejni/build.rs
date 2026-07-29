// 构建脚本：Android 目标链接预编译 librime 静态库及其依赖。
// 库来源：fcitx5-android/prebuilt（NDK 28 构建，minSdk 23），
// 由 scripts 下载到 third_party/librime-android/<abi>/lib。
use std::env;
use std::path::PathBuf;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }
    let abi = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64-v8a",
        Ok("arm") => "armeabi-v7a",
        Ok("x86_64") => "x86_64",
        Ok("x86") => "x86",
        other => panic!("未支持的 Android 架构: {other:?}"),
    };
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let lib_dir = repo_root.join("third_party/librime-android").join(abi).join("lib");
    if !lib_dir.join("librime.a").exists() {
        panic!(
            "缺少预编译 librime：{}，请先执行依赖下载（见 docs/m3-verification.md）",
            lib_dir.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    // 顺序：librime 在前，其依赖随后（静态归档按引用顺序解析）
    for lib in [
        "rime",
        "glog",
        "yaml-cpp",
        "leveldb",
        "marisa",
        "opencc",
        "boost_container",
        "boost_iostreams",
        "boost_random",
        "zstd",
        "iconv",
    ] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    // NDK C++ 静态运行时与系统库
    println!("cargo:rustc-link-lib=static=c++_static");
    println!("cargo:rustc-link-lib=static=c++abi");
    println!("cargo:rustc-link-lib=dylib=log");
    println!("cargo:rustc-link-lib=dylib=z");
}
