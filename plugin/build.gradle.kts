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
    // The Adaptive* preference widgets. Without this there is no way to build
    // a settings screen, and granting stays an adb operation.
    implementation(project(":core:validators"))
    // MaterialAlertDialogBuilder and R.style.DialogTheme — the app's own dialog
    // theme. Without it a dialog built from the activity's dark context renders
    // its own text white on a white Material background: invisible, not absent.
    implementation(project(":core:ui"))
    // Scanning. Declared with a literal coordinate ON PURPOSE: adding it to
    // AAPS's gradle/libs.versions.toml would be a change to the loop app's own
    // build for the sake of an add-on, and the standing constraint on this work
    // is that the rest of that app stays untouched. It brings its own
    // CaptureActivity, so nothing needs registering in the app's manifest.
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    // LoggingWorker, which the sync worker extends.
    implementation(project(":core:objects"))
    // WorkManager: the drain runs as a worker, off the loop's thread.
    api(libs.androidx.work.runtime)

    // Dagger has to run IN THIS MODULE. Declaring @Module and
    // @ContributesAndroidInjector without the processor compiles fine and then
    // fails at the app's KSP step with "SwarmModule_ContributesSwarmDataSyncWorker
    // could not be resolved" — the annotations are read by the component in
    // :app, but the classes they imply are generated here or not at all.
    ksp(libs.com.google.dagger.compiler)
    ksp(libs.com.google.dagger.android.processor)

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

    // EVERY CRATE THE .so LINKS, or gradle will skip a build that mattered.
    //
    // This listed only core and android, so a change confined to
    // diaswarm-net — which is most of the transport, the ALPN included —
    // left the task UP-TO-DATE. The build then "succeeded", the APK
    // installed, and the phone ran the previous native half: it announced an
    // old wire version and every peer was told it "doesn't support any known
    // protocol". Nothing anywhere said the .so was stale.
    //
    // Derived from the directory rather than listed by hand, so a fourth
    // crate cannot be forgotten the same way.
    project.file("../crates").listFiles()?.sorted()?.forEach { c ->
        if (c.resolve("Cargo.toml").exists()) {
            inputs.dir(c.resolve("src"))
            inputs.file(c.resolve("Cargo.toml"))
            if (c.resolve("Cargo.lock").exists()) inputs.file(c.resolve("Cargo.lock"))
        }
    }
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
