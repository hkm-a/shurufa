package com.shurufa.ime

/**
 * Rust JNI 桥（platforms/android/rimejni）的 Kotlin 侧声明。
 * 上下文协议：`preedit \u{1} highlighted \u{1} cursor \u{1} 候选1 \u{1} 候选2 …`，
 * 空组合返回空串。
 */
object RimeBridge {
    init {
        System.loadLibrary("shurufa_rime")
    }

    /** 阻塞完成引擎初始化与方案部署，必须在后台线程调用；幂等。 */
    external fun nativeInit(sharedDir: String, userDir: String): Boolean

    /** X11 keysym 与修饰掩码，与桌面端一致；返回是否被引擎吃掉。 */
    external fun nativeProcessKey(keysym: Int, mask: Int): Boolean

    external fun nativeCommit(): String

    external fun nativeContext(): String

    /** 将引擎内的组合光标同步到编辑器当前的 UTF-16 偏移。 */
    external fun nativeSetCursor(cursorPos: Int)

    /** 候选列表翻页；backward=true 为上一页。返回 `<上下文串>`（同 nativeContext 协议）。 */
    external fun nativeChangePage(backward: Boolean): String

    /** 选择当前页第 `index` 个候选并上屏。返回提交文本（空串=失败/无上屏）。 */
    external fun nativeSelectCandidate(index: Int): String

    external fun nativeReset()

    /** P4-3：主动部署指定方案（编译附加词典，如 radical_pinyin 反查词典）。 */
    external fun nativeDeploySchema(schema: String): Boolean

    /** 切换中英文（ascii_mode），返回切换后是否为英文直输。 */
    external fun nativeToggleAscii(): Boolean

    external fun nativeIsAscii(): Boolean

    /** 引擎状态快照："is_ascii FIELD full_shape FIELD ascii_punct"，取值 "0"/"1"；无会话返回空串。 */
    external fun nativeStatus(): String

    /** 删除当前页第 index 个候选（用户词典词条），成功返回 true。 */
    external fun nativeForgetOnCurrentPage(index: Int): Boolean

    /** 打字统计："totalChars FIELD todayChars FIELD totalKeys FIELD todayKeys"；无数据返回空串。 */
    external fun nativeStatsTotals(): String

    /** 读取当前输入方案（"pinyin" | "double_pinyin" | "wubi" | "cangjie"）。 */
    external fun nativeGetInputScheme(): String

    /** 列出 4 个可选方案 id，逗号分隔。 */
    external fun nativeListInputSchemes(): String

    /** 选择输入方案；写入 options.json + SharedPreferences + 进程内缓存，成功返回 true。 */
    external fun nativeSetInputScheme(ctx: android.content.Context, scheme: String): Boolean
}
