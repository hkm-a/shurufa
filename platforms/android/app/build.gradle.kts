plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.shurufa.ime"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.shurufa.ime"
        // 预编译 librime 面向 platform 23（Android 6.0）
        minSdk = 23
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }

    // 方案数据以仓库根 schemas/ 为唯一来源，构建期拷入 assets，
    // 叠加 Android 专属 default.yaml（只启用袖珍拼音）
    sourceSets {
        getByName("main") {
            assets.srcDir(layout.buildDirectory.dir("generated/schemaAssets"))
        }
    }
}

val syncSchemas = tasks.register<Copy>("syncSchemas") {
    val repoRoot = rootDir.parentFile.parentFile
    from(File(repoRoot, "schemas")) {
        include(
            "pinyin_simp.schema.yaml",
            "pinyin_simp.dict.yaml",
            "stroke.schema.yaml",
            "stroke.dict.yaml",
            "punctuation.yaml",
            "symbols.yaml",
            "key_bindings.yaml",
        )
    }
    from(File(projectDir, "schemas-overlay"))
    into(layout.buildDirectory.dir("generated/schemaAssets/schemas"))
}

tasks.named("preBuild") {
    dependsOn(syncSchemas)
}
