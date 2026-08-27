package dev.lanchat.lan_chat

import android.app.ActivityManager
import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.core.content.ContextCompat
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ForegroundServiceLifecycleInstrumentedTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Before
    fun setUp() {
        context.getSharedPreferences(TEST_PREFERENCES, 0).edit()
            .clear()
            .putBoolean(MainActivity.KEEP_ONLINE, true)
            .commit()
    }

    @After
    fun tearDown() {
        context.getSharedPreferences(TEST_PREFERENCES, 0).edit().clear().commit()
    }

    @Test
    fun userStopIsTemporaryAndKeepsTheOnlineSetting() {
        LanChatForegroundService.recordUserStop(context, TEST_PREFERENCES)

        val preferences = context.getSharedPreferences(TEST_PREFERENCES, 0)
        assertTrue(preferences.getBoolean(MainActivity.KEEP_ONLINE, false))
        assertFalse(LanChatForegroundService.shouldRun(context, TEST_PREFERENCES))

        LanChatForegroundService.clearTemporaryStops(context, TEST_PREFERENCES)
        assertTrue(LanChatForegroundService.shouldRun(context, TEST_PREFERENCES))
    }

    @Test
    fun startingATransferClearsAnEarlierUserStop() {
        LanChatForegroundService.recordUserStop(context, TEST_PREFERENCES)
        LanChatForegroundService.recordActiveTransferCount(context, 1, TEST_PREFERENCES)

        assertTrue(LanChatForegroundService.shouldRun(context, TEST_PREFERENCES))
    }

    @Test
    fun stopNotificationActionStopsServiceWithoutDisablingOnlineSetting() {
        ContextCompat.startForegroundService(
            context,
            android.content.Intent(context, LanChatForegroundService::class.java).apply {
                putExtra(
                    LanChatForegroundService.EXTRA_PREFERENCES_NAME,
                    TEST_PREFERENCES,
                )
            },
        )
        waitUntil { isServiceRunning() }

        val notification = LanChatForegroundService.buildNotification(
            context,
            TEST_PREFERENCES,
        )
        notification.actions.single { it.title.toString() == "停止后台在线" }
            .actionIntent.send()

        waitUntil { !isServiceRunning() }
        val preferences = context.getSharedPreferences(TEST_PREFERENCES, 0)
        assertTrue(preferences.getBoolean(MainActivity.KEEP_ONLINE, false))
        assertFalse(LanChatForegroundService.shouldRun(context, TEST_PREFERENCES))

        LanChatForegroundService.clearTemporaryStops(context, TEST_PREFERENCES)
        LanChatForegroundService.reconcile(context)
    }

    @Suppress("DEPRECATION")
    private fun isServiceRunning(): Boolean =
        context.getSystemService(Context.ACTIVITY_SERVICE)
            .let { it as ActivityManager }
            .getRunningServices(Int.MAX_VALUE)
            .any { it.service.className == LanChatForegroundService::class.java.name }

    private fun waitUntil(condition: () -> Boolean) {
        val deadline = System.nanoTime() + 3_000_000_000L
        while (!condition() && System.nanoTime() < deadline) {
            Thread.sleep(25)
        }
        assertTrue(condition())
    }

    companion object {
        private const val TEST_PREFERENCES = "lan_chat_foreground_service_test"
    }
}
