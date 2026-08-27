package dev.lanchat.lan_chat

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import io.flutter.embedding.engine.FlutterEngineCache
import io.flutter.plugin.common.MethodChannel

class LanChatForegroundService : Service() {
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = notifyNetworkChanged()

        override fun onLost(network: Network) = notifyNetworkChanged()
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        startForeground(NOTIFICATION_ID, buildNotification(this))
        getSystemService(ConnectivityManager::class.java)
            .registerDefaultNetworkCallback(networkCallback)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val preferencesName = intent?.getStringExtra(EXTRA_PREFERENCES_NAME)
            ?: MainActivity.PREFERENCES
        if (intent?.action == ACTION_STOP_ONLINE) {
            recordUserStop(this, preferencesName)
            invokeDart("androidStopOnline")
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
        } else if (intent?.action == ACTION_PAUSE_TRANSFERS) {
            invokeDart("trayPauseAllTransfers")
            getSystemService(NotificationManager::class.java)
                .notify(NOTIFICATION_ID, buildNotification(this, preferencesName))
        } else if (!shouldRun(this, preferencesName)) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
        } else {
            getSystemService(NotificationManager::class.java)
                .notify(NOTIFICATION_ID, buildNotification(this, preferencesName))
        }
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        getSharedPreferences(MainActivity.PREFERENCES, MODE_PRIVATE)
            .edit()
            .putBoolean(PAUSED_BY_SYSTEM_TIMEOUT, true)
            .apply()
        invokeDart("androidSystemOnlineTimeout")
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf(startId)
        showSystemPausedNotification()
    }

    override fun onDestroy() {
        try {
            getSystemService(ConnectivityManager::class.java)
                .unregisterNetworkCallback(networkCallback)
        } catch (_: IllegalArgumentException) {
        }
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun notifyNetworkChanged() {
        mainExecutor.execute { invokeDart("networkChanged") }
    }

    private fun invokeDart(method: String) {
        FlutterEngineCache.getInstance().get(MainActivity.ENGINE_ID)?.let { engine ->
            MethodChannel(
                engine.dartExecutor.binaryMessenger,
                "dev.lanchat/platform",
            ).invokeMethod(method, null)
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val channel = NotificationChannel(
            CHANNEL_ID,
            "局域网连接",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = "保持附近发现、消息和文件传输在线"
            setShowBadge(false)
        }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun showSystemPausedNotification() {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(
                    PAUSED_CHANNEL_ID,
                    "后台连接状态",
                    NotificationManager.IMPORTANCE_DEFAULT,
                ),
            )
        }
        val openApp = PendingIntent.getActivity(
            this,
            3,
            Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        manager.notify(
            PAUSED_NOTIFICATION_ID,
            NotificationCompat.Builder(this, PAUSED_CHANNEL_ID)
                .setSmallIcon(R.mipmap.ic_launcher)
                .setContentTitle("系统已暂停后台连接")
                .setContentText("打开猫猫快传可继续排队中的任务")
                .setContentIntent(openApp)
                .setAutoCancel(true)
                .setCategory(NotificationCompat.CATEGORY_STATUS)
                .build(),
        )
    }

    companion object {
        private const val CHANNEL_ID = "lan_chat_connection"
        private const val PAUSED_CHANNEL_ID = "lan_chat_connection_paused"
        internal const val NOTIFICATION_ID = 53317
        private const val PAUSED_NOTIFICATION_ID = 53316
        internal const val ACTION_PAUSE_TRANSFERS = "dev.lanchat.action.PAUSE_TRANSFERS"
        internal const val ACTION_STOP_ONLINE = "dev.lanchat.action.STOP_ONLINE"
        internal const val EXTRA_PREFERENCES_NAME = "preferences_name"
        private const val ACTIVE_TRANSFER_COUNT = "active_transfer_count"
        private const val STOPPED_BY_USER = "online_stopped_by_user"
        private const val PAUSED_BY_SYSTEM_TIMEOUT = "online_paused_by_system_timeout"

        fun buildNotification(
            context: Context,
            preferencesName: String = MainActivity.PREFERENCES,
        ): Notification {
            val preferences = context.getSharedPreferences(
                preferencesName,
                Context.MODE_PRIVATE,
            )
            val activeTransferCount = preferences.getInt(ACTIVE_TRANSFER_COUNT, 0)
            val summary = if (activeTransferCount > 0) {
                "正在传输 $activeTransferCount 个任务"
            } else {
                "可接收局域网消息和文件"
            }
        val openApp = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val sendClipboard = PendingIntent.getActivity(
            context,
            1,
            Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
                putExtra(MainActivity.EXTRA_NOTIFICATION_ACTION, MainActivity.ACTION_SEND_CLIPBOARD)
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val pauseTransfers = PendingIntent.getService(
            context,
            2,
            Intent(context, LanChatForegroundService::class.java).apply {
                action = ACTION_PAUSE_TRANSFERS
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val stopOnline = PendingIntent.getService(
            context,
            3,
            Intent(context, LanChatForegroundService::class.java).apply {
                action = ACTION_STOP_ONLINE
                if (preferencesName != MainActivity.PREFERENCES) {
                    putExtra(EXTRA_PREFERENCES_NAME, preferencesName)
                }
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("猫猫快传正在保持在线")
            .setContentText(summary)
            .setContentIntent(openApp)
            .addAction(android.R.drawable.ic_menu_send, "发送剪贴板", sendClipboard)
            .addAction(android.R.drawable.ic_media_pause, "暂停全部传输", pauseTransfers)
            .addAction(android.R.drawable.ic_media_pause, "停止后台在线", stopOnline)
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
        }

        internal fun updateActiveTransferCount(context: Context, count: Int) {
            require(count >= 0) { "active transfer count cannot be negative" }
            recordActiveTransferCount(context, count)
            reconcile(context)
        }

        internal fun resumeAfterUserOpenedApp(context: Context) {
            clearTemporaryStops(context)
            context.getSystemService(NotificationManager::class.java)
                .cancel(PAUSED_NOTIFICATION_ID)
            reconcile(context)
        }

        internal fun reconcile(context: Context) {
            if (shouldRun(context)) {
                ContextCompat.startForegroundService(
                    context,
                    Intent(context, LanChatForegroundService::class.java),
                )
            } else {
                context.stopService(Intent(context, LanChatForegroundService::class.java))
            }
        }

        fun recordUserStop(
            context: Context,
            preferencesName: String = MainActivity.PREFERENCES,
        ) {
            context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
                .edit()
                .putBoolean(STOPPED_BY_USER, true)
                .apply()
        }

        fun recordActiveTransferCount(
            context: Context,
            count: Int,
            preferencesName: String = MainActivity.PREFERENCES,
        ) {
            require(count >= 0) { "active transfer count cannot be negative" }
            val editor = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
                .edit()
                .putInt(ACTIVE_TRANSFER_COUNT, count)
            if (count > 0) {
                editor.remove(STOPPED_BY_USER).remove(PAUSED_BY_SYSTEM_TIMEOUT)
            }
            editor.apply()
        }

        fun clearTemporaryStops(
            context: Context,
            preferencesName: String = MainActivity.PREFERENCES,
        ) {
            context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
                .edit()
                .remove(STOPPED_BY_USER)
                .remove(PAUSED_BY_SYSTEM_TIMEOUT)
                .apply()
        }

        fun shouldRun(
            context: Context,
            preferencesName: String = MainActivity.PREFERENCES,
        ): Boolean {
            val preferences = context.getSharedPreferences(
                preferencesName,
                Context.MODE_PRIVATE,
            )
            return ForegroundServicePolicy.shouldRun(
                keepOnline = preferences.getBoolean(MainActivity.KEEP_ONLINE, true),
                activeTransferCount = preferences.getInt(ACTIVE_TRANSFER_COUNT, 0),
                stoppedByUser = preferences.getBoolean(STOPPED_BY_USER, false),
                pausedBySystemTimeout = preferences.getBoolean(
                    PAUSED_BY_SYSTEM_TIMEOUT,
                    false,
                ),
            )
        }

    }
}
