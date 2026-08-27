package dev.lanchat.lan_chat

import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ForegroundNotificationInstrumentedTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun notificationExposesClipboardPauseAndStopActions() {
        val notification = LanChatForegroundService.buildNotification(
            context,
            MainActivity.PREFERENCES,
        )
        val titles = notification.actions.map { it.title.toString() }

        assertEquals(
            listOf("发送剪贴板", "暂停全部传输", "停止后台在线"),
            titles,
        )
        assertTrue(notification.contentIntent != null)
    }

    @Test
    fun clipboardActionRoutesToMainActivityWithAction() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val monitor = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
        val notification = LanChatForegroundService.buildNotification(
            context,
            MainActivity.PREFERENCES,
        )

        val clipboardAction = notification.actions.first()
        assertEquals(context.packageName, clipboardAction.actionIntent.creatorPackage)
        val command = instrumentation.uiAutomation.executeShellCommand(
            "am start -W -n ${context.packageName}/.MainActivity " +
                "--es ${MainActivity.EXTRA_NOTIFICATION_ACTION} ${MainActivity.ACTION_SEND_CLIPBOARD}",
        )
        ParcelFileDescriptor.AutoCloseInputStream(command).bufferedReader().use {
            assertTrue(it.readText().contains("Status: ok"))
        }
        val activity = instrumentation.waitForMonitorWithTimeout(monitor, 10_000)
        assertNotNull("notification action should open MainActivity", activity)
        assertEquals(
            MainActivity.ACTION_SEND_CLIPBOARD,
            activity.intent.getStringExtra(MainActivity.EXTRA_NOTIFICATION_ACTION),
        )
        instrumentation.runOnMainSync { activity.finish() }
        instrumentation.removeMonitor(monitor)
    }
}
