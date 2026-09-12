package dev.lanchat.lan_chat

import android.Manifest
import android.app.AlertDialog
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.webkit.MimeTypeMap
import androidx.core.app.NotificationCompat
import io.flutter.FlutterInjector
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.FlutterEngineCache
import io.flutter.embedding.engine.dart.DartExecutor
import io.flutter.embedding.android.FlutterActivity
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugins.GeneratedPluginRegistrant
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

open class MainActivity : FlutterActivity() {
    private var pendingPickerResult: MethodChannel.Result? = null
    private var pendingPickerKind: String? = null
    private var platformChannel: MethodChannel? = null
    private var clipboardListenerRegistered = false
    private var activityResumed = false
    private var pendingNotificationAction: String? = null
    private var pendingConversationRoute: String? = null
    private var pendingNotificationPermissionResult: MethodChannel.Result? = null
    private var notificationPermissionRequestInFlight = false
    private val safReceiveAdapter by lazy { SafReceiveAdapter(this) }
    private val safSourceAdapter by lazy { SafSourceAdapter(this) }
    private val documentFdAdapter by lazy { DocumentFdAdapter(this) }
    private val composerClipboard by lazy { ComposerClipboard.shared(applicationContext) }
    private val clipboardListener = ClipboardManager.OnPrimaryClipChangedListener {
        platformChannel?.invokeMethod("clipboardChanged", null)
    }

    override fun provideFlutterEngine(context: Context): FlutterEngine {
        FlutterEngineCache.getInstance().get(ENGINE_ID)?.let { return it }

        FlutterInjector.instance().flutterLoader().startInitialization(context)
        FlutterInjector.instance().flutterLoader().ensureInitializationComplete(context, null)
        return FlutterEngine(context.applicationContext).also { engine ->
            registerPlatformChannel(engine)
            GeneratedPluginRegistrant.registerWith(engine)
            engine.dartExecutor.executeDartEntrypoint(
                DartExecutor.DartEntrypoint.createDefault(),
            )
            FlutterEngineCache.getInstance().put(ENGINE_ID, engine)
        }
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        registerPlatformChannel(flutterEngine)
    }

    override fun shouldDestroyEngineWithHost(): Boolean = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        pendingNotificationAction = intent.getStringExtra(EXTRA_NOTIFICATION_ACTION)
        pendingConversationRoute = intent.getStringExtra(EXTRA_OPEN_CONVERSATION)
        LanChatForegroundService.reconcile(this)
        window.decorView.post { explainNotificationPermissionOnce() }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        pendingNotificationAction = intent.getStringExtra(EXTRA_NOTIFICATION_ACTION)
        pendingConversationRoute = intent.getStringExtra(EXTRA_OPEN_CONVERSATION)
    }

    override fun onResume() {
        super.onResume()
        activityResumed = true
        LanChatForegroundService.resumeAfterUserOpenedApp(this)
        platformChannel?.invokeMethod("appResumed", null)
        updateClipboardListener(window.decorView.hasWindowFocus())
        window.decorView.postDelayed({ dispatchNotificationAction() }, 250)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        updateClipboardListener(hasFocus)
    }

    private fun updateClipboardListener(hasWindowFocus: Boolean) {
        val shouldRegister = ClipboardListenerPolicy.shouldRegister(
            activityResumed,
            hasWindowFocus,
        )
        if (shouldRegister && !clipboardListenerRegistered) {
            getSystemService(ClipboardManager::class.java)
                .addPrimaryClipChangedListener(clipboardListener)
            clipboardListenerRegistered = true
        } else if (!shouldRegister && clipboardListenerRegistered) {
            getSystemService(ClipboardManager::class.java)
                .removePrimaryClipChangedListener(clipboardListener)
            clipboardListenerRegistered = false
        }
    }

    override fun onPause() {
        activityResumed = false
        updateClipboardListener(hasWindowFocus = false)
        super.onPause()
    }

    fun isClipboardListenerRegisteredForTesting(): Boolean = clipboardListenerRegistered

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQUEST_NOTIFICATIONS) return
        val granted = grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED
        pendingNotificationPermissionResult?.success(granted)
        pendingNotificationPermissionResult = null
        notificationPermissionRequestInFlight = false
        if (granted) LanChatForegroundService.reconcile(this)
    }

    private fun hasNotificationPermission(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    private fun explainNotificationPermissionOnce() {
        if (hasNotificationPermission()) return
        val preferences = getSharedPreferences(PREFERENCES, MODE_PRIVATE)
        if (preferences.getBoolean(NOTIFICATION_PERMISSION_ASKED, false)) return
        AlertDialog.Builder(this)
            .setTitle("允许显示连接状态")
            .setMessage(
                "通知权限用于显示后台连接状态和快捷操作；拒绝后后台网络仍会继续运行。",
            )
            .setNegativeButton("暂不") { _, _ ->
                preferences.edit().putBoolean(NOTIFICATION_PERMISSION_ASKED, true).apply()
            }
            .setPositiveButton("继续") { _, _ ->
                requestNotificationPermission(null)
            }
            .show()
    }

    private fun requestNotificationPermission(result: MethodChannel.Result?) {
        if (hasNotificationPermission()) {
            result?.success(true)
            return
        }
        if (notificationPermissionRequestInFlight) {
            result?.error("PERMISSION_REQUEST_BUSY", "通知权限请求正在进行", null)
            return
        }
        getSharedPreferences(PREFERENCES, MODE_PRIVATE)
            .edit()
            .putBoolean(NOTIFICATION_PERMISSION_ASKED, true)
            .apply()
        pendingNotificationPermissionResult = result
        notificationPermissionRequestInFlight = true
        requestPermissions(
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            REQUEST_NOTIFICATIONS,
        )
    }

    private fun dispatchNotificationAction() {
        pendingConversationRoute?.let { conversationId ->
            pendingConversationRoute = null
            platformChannel?.invokeMethod("notificationOpenConversation", conversationId)
        }
        val action = pendingNotificationAction ?: return
        pendingNotificationAction = null
        if (action == ACTION_SEND_CLIPBOARD) {
            platformChannel?.invokeMethod("notificationSendClipboard", null)
        }
    }

    private fun registerPlatformChannel(flutterEngine: FlutterEngine) {
        platformChannel = MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "dev.lanchat/platform",
        ).also { channel -> channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "applyAppSettings" -> {
                    val keepOnline = call.argument<Boolean>("androidKeepOnline") == true
                    getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                        .edit()
                        .putBoolean(KEEP_ONLINE, keepOnline)
                        .putBoolean(
                            NOTIFICATIONS_ENABLED,
                            call.argument<Boolean>("notificationsEnabled") != false,
                        )
                        .apply()
                    if (keepOnline) {
                        LanChatForegroundService.resumeAfterUserOpenedApp(this)
                    } else {
                        LanChatForegroundService.reconcile(this)
                    }
                    result.success(null)
                }
                "requestNotificationPermission" -> requestNotificationPermission(result)
                "updateActiveTransferCount" -> {
                    val count = requireNotNull(call.argument<Int>("count"))
                    LanChatForegroundService.updateActiveTransferCount(this, count)
                    result.success(null)
                }
                "openNotificationSettings" -> {
                    startActivity(
                        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).apply {
                            putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
                        },
                    )
                    result.success(null)
                }
                "getDefaultReceiveDirectory" -> result.success(null)
                "moveToBackground" -> {
                    moveTaskToBack(true)
                    result.success(null)
                }
                "openReference" -> {
                    try {
                        openReference(
                            requireNotNull(call.argument<String>("reference")),
                            call.argument<Boolean>("showInFolder") == true,
                        )
                        result.success(null)
                    } catch (error: Throwable) {
                        result.error(
                            "ANDROID_OPEN_REFERENCE_FAILED",
                            error.message ?: error.javaClass.simpleName,
                            null,
                        )
                    }
                }
                "showNotification" -> {
                    showIncomingNotification(
                        requireNotNull(call.argument<String>("title")),
                        requireNotNull(call.argument<String>("body")),
                        call.argument<String>("conversationId"),
                    )
                    result.success(null)
                }
                "getBootstrapInfo" -> runPlatformOperation(result) {
                    val preferences = getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                    mutableMapOf(
                        "dataDirectory" to filesDir.absolutePath,
                        "deviceName" to defaultDeviceName(),
                    ).apply {
                        if (!preferences.getBoolean(DEVICE_NAME_MIGRATED, false)) {
                            put("legacyDeviceName", Build.MODEL)
                        }
                    }
                }
                "completeDeviceNameMigration" -> {
                    getSharedPreferences(PREFERENCES, MODE_PRIVATE).edit()
                        .putBoolean(DEVICE_NAME_MIGRATED, true).apply()
                    result.success(null)
                }
                "pickSource" -> launchSourcePicker(call.argument<String>("kind"), result)
                "readClipboardContent" -> {
                    check(window.decorView.hasWindowFocus()) { "请在前台聊天框中粘贴" }
                    val clip = getSystemService(ClipboardManager::class.java).primaryClip
                    runPlatformOperation(result) { composerClipboard.read(clip) }
                }
                "cancelComposerSource" -> {
                    composerClipboard.cancel(requireNotNull(call.argument<String>("token")))
                    result.success(null)
                }
                "prepareComposerSource" -> {
                    val token = requireNotNull(call.argument<String>("token"))
                    composerClipboard.register(token)
                    runPlatformOperation(result) {
                        composerClipboard.prepare(Uri.parse(requireNotNull(call.argument<String>("uri"))),
                            requireNotNull(call.argument<String>("kind")), token,
                            requireNotNull(call.argument<String>("session"))) { bytes, total ->
                            runOnUiThread { platformChannel?.invokeMethod("composerSourceProgress",
                                mapOf("token" to token, "bytes" to bytes, "total" to total)) }
                        }
                    }
                }
                "validateComposerSources" -> runPlatformOperation(result) {
                    for (reference in requireNotNull(call.argument<List<String>>("uris"))) {
                        contentResolver.openFileDescriptor(Uri.parse(reference), "r")?.use { }
                            ?: error("附件权限已失效，请重新选择")
                    }
                    null
                }
                "readClipboardImage" -> runPlatformOperation(result) {
                    readClipboardImage()
                }
                "cacheImagePreview" -> runPlatformOperation(result) {
                    cacheImagePreview(
                        Uri.parse(requireNotNull(call.argument<String>("uri"))),
                    )
                }
                "writeClipboardImage" -> {
                    try {
                        writeClipboardImage(
                            requireNotNull(call.argument<String>("reference")),
                            requireNotNull(call.argument<String>("metadata")),
                            requireNotNull(call.argument<String>("fingerprint")),
                        )
                        result.success(null)
                    } catch (error: Throwable) {
                        result.error(
                            "ANDROID_CLIPBOARD_IMAGE_FAILED",
                            error.message ?: error.javaClass.simpleName,
                            null,
                        )
                    }
                }
                "pickReceiveDirectory" -> launchReceiveDirectoryPicker(result)
                "getSavedReceiveTree" -> result.success(
                    getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                        .getString(SAVED_RECEIVE_TREE, null),
                )
                "clearSavedReceiveTree" -> {
                    getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                        .edit()
                        .remove(SAVED_RECEIVE_TREE)
                        .apply()
                    result.success(null)
                }
                "prepareReceiveTree" -> runPlatformOperation(result) {
                    safReceiveAdapter.prepareReceiveTree(
                        Uri.parse(requireNotNull(call.argument<String>("treeUri"))),
                        requireNotNull(call.argument<List<Map<String, Any?>>>("entries")),
                    )
                }
                "openDocumentFd" -> runPlatformOperation(result) {
                    val uri = Uri.parse(requireNotNull(call.argument<String>("uri")))
                    val writable = call.argument<Boolean>("writable") == true
                    mapOf("fd" to documentFdAdapter.open(uri, writable))
                }
                "closeRawFd" -> {
                    val fd = requireNotNull(call.argument<Int>("fd"))
                    documentFdAdapter.close(fd)
                    result.success(null)
                }
                "commitReceiveFile" -> runPlatformOperation(result) {
                    val requestedUri = Uri.parse(requireNotNull(call.argument<String>("uri")))
                    val finalName = requireNotNull(call.argument<String>("finalName"))
                    val renamed = safReceiveAdapter.commitReceiveFile(requestedUri, finalName)
                    mapOf("uri" to renamed.toString())
                }
                else -> result.notImplemented()
            }
        } }
    }

    @Deprecated("Activity result API is intentionally kept dependency-free for the cached engine")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_PICK_SOURCE && requestCode != REQUEST_PICK_RECEIVE_TREE) return
        val result = pendingPickerResult ?: return
        val kind = pendingPickerKind
        pendingPickerResult = null
        pendingPickerKind = null
        if (resultCode != RESULT_OK || data?.data == null) {
            result.success(null)
            return
        }
        val uri = requireNotNull(data.data)
        val flags = data.flags and
            (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        try {
            contentResolver.takePersistableUriPermission(uri, flags)
        } catch (_: SecurityException) {
            // Some providers only grant access for the current process lifetime.
        }
        if (requestCode == REQUEST_PICK_RECEIVE_TREE) {
            runPlatformOperation(result) {
                safReceiveAdapter.probeReceiveTree(uri)
                getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                    .edit()
                    .putString(SAVED_RECEIVE_TREE, uri.toString())
                    .apply()
                mapOf("treeUri" to uri.toString())
            }
        } else {
            runPlatformOperation(result) {
                safSourceAdapter.enumerate(uri, requireNotNull(kind))
            }
        }
    }

    private fun launchSourcePicker(kind: String?, result: MethodChannel.Result) {
        if (kind !in setOf("file", "image", "folder")) {
            result.error("INVALID_SOURCE_KIND", "不支持的附件类型", kind)
            return
        }
        if (pendingPickerResult != null) {
            result.error("PICKER_BUSY", "已有文件选择器正在显示", null)
            return
        }
        pendingPickerResult = result
        pendingPickerKind = kind
        val intent = if (kind == "folder") {
            Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
        } else {
            Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = if (kind == "image") "image/*" else "*/*"
            }
        }.apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
        }
        startActivityForResult(intent, REQUEST_PICK_SOURCE)
    }

    private fun showIncomingNotification(
        title: String,
        body: String,
        conversationId: String?,
    ) {
        if (!getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                .getBoolean(NOTIFICATIONS_ENABLED, true)
        ) return
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(
                    INCOMING_CHANNEL,
                    "消息和传输",
                    NotificationManager.IMPORTANCE_DEFAULT,
                ),
            )
        }
        val open = PendingIntent.getActivity(
            this,
            conversationId?.hashCode() ?: title.hashCode(),
            Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
                putExtra(EXTRA_OPEN_CONVERSATION, conversationId)
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = NotificationCompat.Builder(this, INCOMING_CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setContentIntent(open)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .build()
        manager.notify(conversationId?.hashCode() ?: title.hashCode(), notification)
    }

    private fun launchReceiveDirectoryPicker(result: MethodChannel.Result) {
        if (pendingPickerResult != null) {
            result.error("PICKER_BUSY", "已有文件选择器正在显示", null)
            return
        }
        pendingPickerResult = result
        pendingPickerKind = "receive"
        startActivityForResult(
            Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
                addFlags(
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                        Intent.FLAG_GRANT_PREFIX_URI_PERMISSION,
                )
            },
            REQUEST_PICK_RECEIVE_TREE,
        )
    }

    private fun readClipboardImage(): Map<String, Any>? {
        val clipboard = getSystemService(ClipboardManager::class.java)
        val clip = clipboard.primaryClip ?: return null
        if (clip.itemCount == 0 || !clip.description.hasMimeType("image/*")) return null
        val source = clip.getItemAt(0).uri ?: return null
        val mimeType = contentResolver.getType(source) ?: "image/png"
        val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mimeType) ?: "img"
        val directory = File(filesDir, "clipboard-sources").apply { mkdirs() }
        val destination = File(directory, "clipboard-${System.currentTimeMillis()}.$extension")
        var total = 0L
        val digest = MessageDigest.getInstance("SHA-256")
        try {
            contentResolver.openInputStream(source).use { input ->
                requireNotNull(input) { "无法读取剪贴板图片" }
                FileOutputStream(destination).use { output ->
                    val buffer = ByteArray(256 * 1024)
                    while (true) {
                        val read = input.read(buffer)
                        if (read < 0) break
                        total += read
                        require(total <= MAX_CLIPBOARD_IMAGE_BYTES) {
                            "剪贴板图片超过 20 MiB"
                        }
                        digest.update(buffer, 0, read)
                        output.write(buffer, 0, read)
                    }
                    output.fd.sync()
                }
            }
        } catch (error: Throwable) {
            destination.delete()
            throw error
        }
        if (total == 0L) {
            destination.delete()
            error("剪贴板图片为空")
        }
        val fingerprint = digest.digest().joinToString("") {
            "%02x".format(it.toInt() and 0xff)
        }
        val preferences = getSharedPreferences(PREFERENCES, MODE_PRIVATE)
        val suppressSync = preferences.getString(CLIPBOARD_IMAGE_FINGERPRINT, null) == fingerprint
        if (suppressSync) {
            preferences.edit()
                .remove(CLIPBOARD_IMAGE_FINGERPRINT)
                .remove(CLIPBOARD_IMAGE_METADATA)
                .apply()
        }
        return mapOf(
            "displayName" to destination.name,
            "sourceRef" to destination.absolutePath,
            "relativePath" to destination.name,
            "size" to total,
            "modifiedAtMs" to destination.lastModified(),
            "fingerprint" to fingerprint,
            "suppressSync" to suppressSync,
        )
    }

    private fun writeClipboardImage(
        reference: String,
        metadata: String,
        fingerprint: String,
    ) {
        val uri = Uri.parse(reference)
        require(uri.scheme == "content") { "Android 剪贴板图片必须使用 content URI" }
        require(fingerprint.matches(Regex("[0-9a-f]{64}"))) { "图片指纹格式无效" }
        contentResolver.openFileDescriptor(uri, "r")?.use { } ?: error("接收图片不可读取")
        check(
            getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                .edit()
                .putString(CLIPBOARD_IMAGE_FINGERPRINT, fingerprint)
                .putString(CLIPBOARD_IMAGE_METADATA, metadata)
                .commit(),
        ) { "无法保存剪贴板来源" }
        getSystemService(ClipboardManager::class.java).setPrimaryClip(
            ClipData.newUri(contentResolver, "LAN Chat image", uri),
        )
    }

    private fun openReference(reference: String, showInFolder: Boolean) {
        val receiveTreeReference = if (showInFolder) {
            getSharedPreferences(PREFERENCES, MODE_PRIVATE)
                .getString(SAVED_RECEIVE_TREE, null)
        } else {
            null
        }
        val intent = buildOpenReferenceIntent(
            this,
            reference,
            showInFolder,
            receiveTreeReference,
        )
        startActivity(intent)
    }

    private fun cacheImagePreview(uri: Uri): String {
        require(uri.scheme == "content") { "缩略图来源必须是 content URI" }
        val key = MessageDigest.getInstance("SHA-256")
            .digest(uri.toString().toByteArray(Charsets.UTF_8))
            .joinToString("") { byte -> "%02x".format(byte) }
        val previewDirectory = File(cacheDir, "image-previews").apply {
            require(mkdirs() || isDirectory) { "无法创建图片缓存目录" }
        }
        val preview = File(previewDirectory, key)
        if (preview.isFile && preview.length() > 0L) return preview.absolutePath

        val temporary = File(previewDirectory, ".$key.tmp")
        try {
            contentResolver.openInputStream(uri).use { input ->
                requireNotNull(input) { "文档提供器无法读取图片" }
                FileOutputStream(temporary).use { output ->
                    input.copyTo(output, DEFAULT_BUFFER_SIZE)
                    output.fd.sync()
                }
            }
            require(temporary.length() > 0L) { "图片内容为空" }
            if (!temporary.renameTo(preview)) {
                temporary.copyTo(preview, overwrite = true)
                temporary.delete()
            }
            return preview.absolutePath
        } finally {
            if (temporary.exists()) temporary.delete()
        }
    }

    private fun defaultDeviceName(): String {
        val marketNames = listOf(
            "ro.product.marketname",
            "ro.product.vendor.marketname",
            "ro.product.odm.marketname",
        ).map(::readDeviceProperty)
        val systemName = try {
            Settings.Global.getString(contentResolver, "device_name")
        } catch (_: SecurityException) {
            null
        }
        return DeviceNamePolicy.chooseDefaultName(Build.MODEL, marketNames, systemName)
    }

    private fun readDeviceProperty(name: String): String {
        val process = try {
            ProcessBuilder("/system/bin/getprop", name).start()
        } catch (_: Exception) {
            return ""
        }
        return try {
            if (process.waitFor(250, TimeUnit.MILLISECONDS) && process.exitValue() == 0) {
                process.inputStream.bufferedReader().use { it.readText().trim() }
            } else {
                ""
            }
        } catch (_: Exception) {
            ""
        } finally {
            process.destroy()
        }
    }

    private fun runPlatformOperation(
        result: MethodChannel.Result,
        operation: () -> Any?,
    ) {
        Thread {
            try {
                val value = operation()
                runOnUiThread { result.success(value) }
            } catch (error: Throwable) {
                runOnUiThread {
                    result.error(
                        androidPlatformErrorCode(error),
                        error.message ?: error.javaClass.simpleName,
                        null,
                    )
                }
            }
        }.start()
    }

    companion object {
        internal const val ENGINE_ID = "lan-chat-engine"
        internal const val EXTRA_NOTIFICATION_ACTION = "notification_action"
        internal const val ACTION_SEND_CLIPBOARD = "send_clipboard"
        private const val EXTRA_OPEN_CONVERSATION = "open_conversation"
        private const val REQUEST_PICK_SOURCE = 53319
        private const val REQUEST_PICK_RECEIVE_TREE = 53320
        private const val REQUEST_NOTIFICATIONS = 53321
        private const val MAX_CLIPBOARD_IMAGE_BYTES = 20L * 1024L * 1024L
        internal const val PREFERENCES = "lan_chat_platform"
        private const val SAVED_RECEIVE_TREE = "saved_receive_tree"
        private const val DEVICE_NAME_MIGRATED = "readable_device_name_migrated"
        internal const val KEEP_ONLINE = "keep_online"
        private const val NOTIFICATIONS_ENABLED = "notifications_enabled"
        private const val NOTIFICATION_PERMISSION_ASKED = "notification_permission_asked"
        private const val CLIPBOARD_IMAGE_FINGERPRINT = "clipboard_image_fingerprint"
        private const val CLIPBOARD_IMAGE_METADATA = "clipboard_image_metadata"
        private const val INCOMING_CHANNEL = "lan_chat_incoming"
    }
}
