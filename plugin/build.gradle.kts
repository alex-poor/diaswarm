plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.ksp)
    id("kotlin-android")
    id("android-module-dependencies")
    id("test-module-dependencies")
    id("jacoco-module-dependencies")
}

android {
    namespace = "app.aaps.plugins.sync.swarm"

    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

    packaging {
        // The .so is built by cargo-ndk below and copied in; nothing here
        // compiles C, so no externalNativeBuild and no CMake.
        jniLibs.useLegacyPackaging = false
    }
}

dependencies {
    implementation(project(":core:data"))
    implementation(project(":core:interfaces"))
    implementation(project(":core:keys"))
    implementation(project(":core:utils"))

    testImplementation(project(":shared:tests"))
}

/**
 * Build the record core for Android and drop the .so where the library packages
 * it from.
 *
 * WHY A GRADLE TASK RATHER THAN CHECKED-IN BINARIES. A committed .so is a
 * second, opaque copy of the frozen spec that nobody can diff and everybody
 * trusts. Building it here means the Kotlin and the Rust cannot drift without
 * the build noticing — which is the same argument that put the record logic in
 * one crate to begin with.
 *
 * It needs `cargo-ndk` and an NDK. If neither is present the task fails with a
 * message saying so, rather than silently producing a plugin whose native half
 * is missing and which throws UnsatisfiedLinkError on a loop phone.
 */
val abis = listOf("arm64-v8a", "armeabi-v7a")

val buildRustCore = tasks.register<Exec>("buildRustCore") {
    group = "build"
    description = "Cross-compile crates/diaswarm-android for $abis"

    val crate = project.file("../crates/diaswarm-android")
    workingDir = crate
    commandLine(
        buildList {
            add("cargo")
            add("ndk")
            abis.forEach { add("-t"); add(it) }
            add("-o"); add(project.file("src/main/jniLibs").absolutePath)
            add("build")
            add("--release")
        }
    )

    inputs.dir(crate.resolve("src"))
    inputs.dir(project.file("../crates/diaswarm-core/src"))
    inputs.file(crate.resolve("Cargo.toml"))
    outputs.dir(project.file("src/main/jniLibs"))

    doFirst {
        val ndk = System.getenv("ANDROID_NDK_HOME") ?: System.getenv("NDK_HOME")
        require(!ndk.isNullOrBlank()) {
            "ANDROID_NDK_HOME is not set. The swarm plugin needs the NDK to build " +
                "the record core; without it the plugin would ship without its " +
                "native half and fail at load."
        }
    }
}

tasks.named("preBuild") { dependsOn(buildRustCore) }
