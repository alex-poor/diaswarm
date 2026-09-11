plugins {
    id("com.android.application") version "8.13.2"
    id("org.jetbrains.kotlin.android") version "2.2.21"
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.21"
}

android {
    namespace = "nz.diaswarm.follower"
    compileSdk = 36

    defaultConfig {
        // PERMANENT ONCE PUBLISHED. Changing an applicationId after a release
        // makes every existing install a stranger: no update path, uninstall
        // and start again. Derived from a domain rather than invented, and
        // deliberately nothing like `info.nightscout.*` — this is not AAPS and
        // should not be mistaken for it in a launcher or a package list.
        applicationId = "nz.diaswarm.ayni"
        minSdk = 31
        targetSdk = 36

        // MONOTONIC, AND ITS OWN. The loop app's versionCode is a constant
        // shared across its flavours; a published app needs a number that only
        // ever goes up, or the second release cannot be installed over the
        // first.
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            // R8, because 22 MB of dex for one screen is mostly Compose and
            // zxing that this app never calls. Verified on a device rather than
            // trusted: shrinking is exactly the change that compiles and then
            // fails at runtime on a reflective call nobody kept.
            isMinifyEnabled = true
            isShrinkResources = true
            setProguardFiles(listOf(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro"))
            // No signingConfig: CI signs the output with a key held in secrets,
            // so nothing in this file can reach for a keystore.
        }
    }

    buildFeatures { compose = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
    kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21) } }

    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
    packaging {
        jniLibs {
            useLegacyPackaging = false
            // NOT LINKED, AND NOT SMALL. cargo-ndk copies every .so it finds,
            // and several dependencies build a cdylib nobody loads: checked
            // with `readelf -d`, libdiaswarm_android needs only libc, libm and
            // libdl. About 1.2 MB per ABI of library that is downloaded,
            // installed and never opened.
            excludes += listOf("**/libiroh*.so", "**/libirpc*.so")
        }
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.12.2")
    implementation(platform("androidx.compose:compose-bom:2025.12.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.work:work-runtime-ktx:2.10.0")
    // Scanning and encoding an invite. Maven Central, not jitpack.
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}

/**
 * Build the record core for Android and drop the .so where the app packages it.
 *
 * The same argument as the AAPS add-on: a committed .so is a second, opaque
 * copy of the frozen spec that nobody can diff and everybody trusts. Building
 * it here means the Kotlin and the Rust cannot drift without the build noticing
 * — and F-Droid's scanner rejects prebuilt binaries in a source tree anyway.
 */
val abis = listOf("arm64-v8a", "armeabi-v7a")

val buildRustCore = tasks.register<Exec>("buildRustCore") {
    group = "build"
    description = "Cross-compile crates/diaswarm-android for $abis"
    val crate = project.file("../crates/diaswarm-android")
    workingDir = crate
    commandLine(buildList {
        add("cargo"); add("ndk")
        abis.forEach { add("-t"); add(it) }
        add("-o"); add(project.file("src/main/jniLibs").absolutePath)
        add("build"); add("--release")
    })
    // EVERY CRATE THE .so LINKS, derived rather than listed, so a new one
    // cannot be forgotten and leave the build reporting UP-TO-DATE while the
    // phone runs the previous native half.
    project.file("../crates").listFiles()?.sorted()?.forEach { c ->
        if (c.resolve("Cargo.toml").exists()) {
            inputs.dir(c.resolve("src"))
            inputs.file(c.resolve("Cargo.toml"))
            if (c.resolve("Cargo.lock").exists()) inputs.file(c.resolve("Cargo.lock"))
        }
    }
    outputs.dir(project.file("src/main/jniLibs"))
    doFirst {
        require(!System.getenv("ANDROID_NDK_HOME").isNullOrBlank() || !System.getenv("NDK_HOME").isNullOrBlank()) {
            "ANDROID_NDK_HOME is not set. The follower needs the NDK to build the record core; " +
                "without it the app would ship with no native half and fail at load."
        }
    }
}

tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }
    .configureEach { dependsOn(buildRustCore) }
