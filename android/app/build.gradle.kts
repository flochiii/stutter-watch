plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

val nativeAbis = providers.gradleProperty("androidAbis").orElse("armeabi-v7a,arm64-v8a").get().split(',')
android {
    namespace = "io.github.flochiii.stutterwatch"
    compileSdk = 37
    ndkVersion = "28.2.13676358"
    defaultConfig {
        applicationId = "io.github.flochiii.stutterwatch"
        minSdk = 29
        targetSdk = 37
        versionCode = 3
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    buildFeatures { compose = true; aidl = true; buildConfig = true }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    flavorDimensions += "architecture"
    productFlavors {
        create("universal") { dimension = "architecture"; ndk { abiFilters += nativeAbis } }
        create("arm64") { dimension = "architecture"; ndk { abiFilters += "arm64-v8a" } }
    }
    signingConfigs {
        create("distribution") {
            val keyPath = System.getenv("ANDROID_SIGNING_STORE")
            if (!keyPath.isNullOrEmpty()) {
                storeFile = file(keyPath)
                storePassword = System.getenv("ANDROID_SIGNING_PASSWORD")
                keyAlias = System.getenv("ANDROID_SIGNING_ALIAS") ?: "stutter-watch"
                keyPassword = System.getenv("ANDROID_SIGNING_KEY_PASSWORD") ?: storePassword
            }
        }
    }
    buildTypes {
        debug { applicationIdSuffix = ".debug" }
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("distribution")
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    sourceSets.getByName("main") {
        jniLibs.directories.add(layout.buildDirectory.dir("generated/jniLibs").get().asFile.absolutePath)
        assets.directories.add(layout.buildDirectory.dir("generated/assets").get().asFile.absolutePath)
    }
    packaging { resources.excludes += setOf("META-INF/AL2.0", "META-INF/LGPL2.1") }
    testOptions { unitTests.isReturnDefaultValues = true }
}

val nativeBuild by tasks.registering(Exec::class) {
    workingDir(rootDir.parentFile)
    commandLine(providers.environmentVariable("PYTHON").orElse("python").get(), "scripts/build-android-native.py", "--abis", nativeAbis.joinToString(","))
    environment("ANDROID_HOME", androidComponents.sdkComponents.sdkDirectory.get().asFile.absolutePath)
    inputs.files(fileTree("../../src"), file("../../Cargo.toml"), file("../../Cargo.lock"), fileTree("../native/src"))
    outputs.dir(layout.buildDirectory.dir("generated/jniLibs"))
}
val appAssets by tasks.registering(Copy::class) {
    from("../../locales") { into("locales") }
    from("../../assets/fonts/NotoSans-Regular.ttf") { into("fonts") }
    from("../../assets/fonts/OFL-NotoSans.txt") { into("licenses") }
    from("../../LICENSE")
    from("../../THIRD_PARTY_NOTICES.md")
    into(layout.buildDirectory.dir("generated/assets"))
}
tasks.named("preBuild") { dependsOn(nativeBuild, appAssets) }

dependencies {
    implementation(platform("androidx.compose:compose-bom:2025.08.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.2")
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("dev.rikka.shizuku:api:13.1.5")
    implementation("dev.rikka.shizuku:provider:13.1.5")
    implementation("com.github.topjohnwu.libsu:core:6.0.0")
    implementation("com.github.topjohnwu.libsu:service:6.0.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
}
