package com.shurufa.ime

/**
 * Rust JNI 桥（platforms/android/rimejni）的 Kotlin 侧声明。
 * 上下文协议：`preedit  highlighted  候选1  候选2 …`，
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

    external fun nativeReset()
}
