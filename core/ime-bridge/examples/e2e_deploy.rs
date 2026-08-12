use ime_bridge::Engine;
use std::path::PathBuf;
fn main() {
    // 完全模拟生产：ProgramData schemas + %APPDATA%\shurufa\rime user-dir
    let shared = PathBuf::from("C:/ProgramData/shurufa/schemas");
    let ud = PathBuf::from(env!("APPDATA")).join("shurufa").join("rime");
    std::fs::create_dir_all(&ud).unwrap();
    let e: &'static Engine = Box::leak(Box::new(Engine::init(&shared, &ud).unwrap()));
    let s = e.create_session().unwrap();
    // 模拟 algo 的 create_session_with_scheme：切到 options.json 指定的方案
    let ok = s.select_schema("shurufa_double_pinyin");
    println!("select_schema(双拼) = {ok}");
    s.simulate("nihc ");
    let committed = s.commit().unwrap_or_default();
    println!("nihc 空格 → 上屏: {committed:?}");
    assert_eq!(committed, "你好", "双拼上屏不符");
    println!("E2E 部署环境双拼验证通过 ✅");
}
