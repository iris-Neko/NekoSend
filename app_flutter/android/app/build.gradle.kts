import java.util.Properties
import java.util.zip.ZipFile

val releaseSigningProperties = Properties()
val releaseSigningFile = rootProject.file("key.properties")
if (releaseSigningFile.isFile) {
    releaseSigningFile.inputStream().use(releaseSigningProperties::load)
}

fun releaseSigningValue(property: String, environment: String): String? =
    releaseSigningProperties.getProperty(property)?.takeIf(String::isNotBlank)
        ?: System.getenv(environment)?.takeIf(String::isNotBlank)

val releaseStoreFile = releaseSigningValue("storeFile", "LAN_CHAT_ANDROID_STORE_FILE")
val releaseStorePassword = releaseSigningValue(
    "storePassword",
    "LAN_CHAT_ANDROID_STORE_PASSWORD",
)
val releaseKeyAlias = releaseSigningValue("keyAlias", "LAN_CHAT_ANDROID_KEY_ALIAS")
val releaseKeyPassword = releaseSigningValue(
    "keyPassword",
    "LAN_CHAT_ANDROID_KEY_PASSWORD",
)
val releaseSigningValues = listOf(
    releaseStoreFile,
    releaseStorePassword,
    releaseKeyAlias,
    releaseKeyPassword,
)
val hasReleaseSigning = releaseSigningValues.all { it != null }
if (releaseSigningValues.any { it != null } && !hasReleaseSigning) {
    throw GradleException(
        "Android release signing is only partially configured. Provide storeFile, " +
            "storePassword, keyAlias, and keyPassword together.",
    )
}
val allowDebugReleaseSigning = providers.gradleProperty(
    "lanChatAllowDebugReleaseSigning",
).orNull?.toBooleanStrictOrNull() == true ||
    System.getenv("LAN_CHAT_ALLOW_DEBUG_RELEASE_SIGNING")?.toBooleanStrictOrNull() == true

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "dev.lanchat.lan_chat"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "dev.lanchat.lan_chat"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = 33
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = rootProject.file(requireNotNull(releaseStoreFile))
                storePassword = requireNotNull(releaseStorePassword)
                keyAlias = requireNotNull(releaseKeyAlias)
                keyPassword = requireNotNull(releaseKeyPassword)
            }
        }
    }

    buildTypes {
        release {
            signingConfig = when {
                hasReleaseSigning -> signingConfigs.getByName("release")
                allowDebugReleaseSigning -> signingConfigs.getByName("debug")
                else -> null
            }
        }
    }

    packaging {
        resources {
            excludes += "META-INF/DEPENDENCIES"
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test:core-ktx:1.6.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit-ktx:1.2.1")
    androidTestImplementation("org.apache.sshd:sshd-sftp:2.15.0")
}

// AGP 9 compatibility mode omits Java unit-test output from AndroidUnitTest's
// worker classpath. Run the same compiled tests with Gradle's standard runner.
afterEvaluate {
    val verifyReleaseSigning = tasks.register("verifyReleaseSigningConfiguration") {
        group = "verification"
        doLast {
            if (!hasReleaseSigning && !allowDebugReleaseSigning) {
                throw GradleException(
                    "Android release signing is not configured. Set the four " +
                        "LAN_CHAT_ANDROID_* environment variables or android/key.properties. " +
                        "For a non-publishable local verification build only, set " +
                        "LAN_CHAT_ALLOW_DEBUG_RELEASE_SIGNING=true or pass " +
                        "-PlanChatAllowDebugReleaseSigning=true to Gradle.",
                )
            }
        }
    }
    tasks.named("assembleRelease") {
        dependsOn(verifyReleaseSigning)
    }

    val javaTestClasses = layout.buildDirectory
        .dir("intermediates/javac/debugUnitTest/compileDebugUnitTestJavaWithJavac/classes")
        .get()
        .asFile
    val mainRuntimeJar = layout.buildDirectory
        .file("intermediates/runtime_app_classes_jar/debug/bundleDebugClassesToRuntimeJar/classes.jar")
        .get()
        .asFile
    val compatibilityTest = tasks.register<JavaExec>(
        "testDebugUnitTestCompatibility",
    ) {
        group = "verification"
        description = "Runs debug JVM tests around the AGP 9 compatibility classpath issue."
        dependsOn("compileDebugUnitTestJavaWithJavac", "bundleDebugClassesToRuntimeJar")
        classpath = files(javaTestClasses, mainRuntimeJar) +
            configurations.getByName("debugUnitTestRuntimeClasspath")
        mainClass = "org.junit.runner.JUnitCore"
        args("dev.lanchat.lan_chat.ForegroundServicePolicyTest")
    }
    tasks.named<org.gradle.api.tasks.testing.Test>("testDebugUnitTest") {
        dependsOn(compatibilityTest)
        enabled = false
    }

    fun registerRustLibraryVerification(variant: String) {
        val variantLower = variant.lowercase()
        val verification = tasks.register("verify${variant}RustNativeLibrary") {
            group = "verification"
            val apk = layout.buildDirectory.file(
                "outputs/apk/$variantLower/app-$variantLower.apk",
            )
            inputs.file(apk)
            doLast {
                val apkFile = apk.get().asFile
                if (!apkFile.isFile) {
                    throw GradleException("APK was not produced: $apkFile")
                }
                val targetPlatforms = providers.gradleProperty("target-platform")
                    .orElse("android-arm64")
                    .get()
                    .split(',')
                val targetPlatformToAbi = mapOf(
                    "android-arm64" to "arm64-v8a",
                    "android-x64" to "x86_64",
                )
                val libraryPaths = targetPlatforms.map { platform ->
                    val abi = targetPlatformToAbi[platform]
                        ?: throw GradleException(
                            "Unsupported Android target platform for the Rust core: $platform",
                        )
                    "lib/$abi/liblan_chat_core.so"
                }
                ZipFile(apkFile).use { archive ->
                    libraryPaths.forEach { libraryPath ->
                        if (archive.getEntry(libraryPath) == null) {
                            throw GradleException(
                                "APK is missing $libraryPath. Remove .dart_tool/flutter_build " +
                                    "and rebuild so the Rust native-assets hook runs again.",
                            )
                        }
                    }
                }
            }
        }
        tasks.named("assemble$variant") {
            finalizedBy(verification)
        }
    }

    registerRustLibraryVerification("Debug")
    registerRustLibraryVerification("Release")
}
