plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// 版本单一事实源是仓库根 version.json；SHURUFA_VERSION_* gradle 属性仍可覆盖（用于本地试包）。
val repoRootDir = rootDir.parentFile.parentFile
val shurufaVersionJson = groovy.json.JsonSlurper()
    .parseText(File(repoRootDir, "version.json").readText(Charsets.UTF_8)) as Map<*, *>
val shurufaVersionCode = providers.gradleProperty("SHURUFA_VERSION_CODE")
    .orElse((shurufaVersionJson["versionCode"] as Number).toString())
    .get().toInt()
val shurufaVersionName = providers.gradleProperty("SHURUFA_VERSION_NAME")
    .orElse(shurufaVersionJson["version"] as String)
    .get()

android {
    namespace = "com.shurufa.ime"
    compileSdk = 35
    buildFeatures {
        buildConfig = true
    }

    defaultConfig {
        applicationId = "com.shurufa.ime"
        // 预编译 librime 面向 platform 23（Android 6.0）
        minSdk = 23
        targetSdk = 35
        versionCode = shurufaVersionCode
        versionName = shurufaVersionName
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
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
    // 叠加 Android 专属 default.yaml（只启用雾凇拼音）。
    sourceSets {
        getByName("main") {
            assets.srcDir(layout.buildDirectory.dir("generated/schemaAssets"))
        }
    }
}

val syncSchemas = tasks.register<Copy>("syncSchemas") {
    from(File(repoRootDir, "schemas")) {
        include(
            "rime_ice.schema.yaml",
            "rime_ice.dict.yaml",
            "cn_dicts/8105.dict.yaml",
            "cn_dicts/base.dict.yaml",
            "cn_dicts/ext.dict.yaml",
            "cn_dicts/others.dict.yaml",
            "punctuation.yaml",
            "symbols.yaml",
            "key_bindings.yaml",
            "shurufa-skin.json",
            "rime-ice-2026.06.30.json",
        )
    }
    from(File(projectDir, "schemas-overlay"))
    into(layout.buildDirectory.dir("generated/schemaAssets/schemas"))
}

tasks.named("preBuild") {
    dependsOn(syncSchemas)
}

val copyVersionedDebugApk = tasks.register<Copy>("copyVersionedDebugApk") {
    from(layout.buildDirectory.file("outputs/apk/debug/app-debug.apk"))
    into(layout.buildDirectory.dir("outputs/apk/versioned"))
    rename { "shurufa-${shurufaVersionName}-${shurufaVersionCode}-debug.apk" }
}

tasks.matching { it.name == "assembleDebug" }.configureEach {
    finalizedBy(copyVersionedDebugApk)
}

dependencies {
    // FileProvider 与 InputConnectionCompat.commitContent（图片上屏）
    implementation("androidx.core:core:1.13.1")
    implementation("org.json:json:20240303")
    testImplementation("junit:junit:4.13.2")
}
