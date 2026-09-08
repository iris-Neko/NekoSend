package dev.lanchat.lan_chat

import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.FlutterEngineCache
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ActivityLifecycleInstrumentedTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun recreationKeepsEngineAndClipboardListenerTracksForegroundState() {
        var originalEngine: FlutterEngine? = null
        val firstMonitor = instrumentation.addMonitor(
            TestLifecycleActivity::class.java.name,
            null,
            false,
        )
        shellStartTestActivity()
        val first = instrumentation.waitForMonitorWithTimeout(firstMonitor, 10_000)
        assertNotNull("ADB shell should launch the test activity while locked", first)
        first as TestLifecycleActivity
        instrumentation.waitForIdleSync()
        assertEquals(
            first.window.decorView.hasWindowFocus(),
            first.isClipboardListenerRegisteredForTesting(),
        )
        originalEngine = FlutterEngineCache.getInstance().get(MainActivity.ENGINE_ID)
        assertTrue(originalEngine != null)

        instrumentation.runOnMainSync { first.moveTaskToBack(true) }
        instrumentation.waitForIdleSync()
        assertFalse(first.isFinishing)
        assertSame(originalEngine, FlutterEngineCache.getInstance().get(MainActivity.ENGINE_ID))
        assertFalse(first.isClipboardListenerRegisteredForTesting())

        val secondMonitor = instrumentation.addMonitor(
            TestLifecycleActivity::class.java.name,
            null,
            false,
        )
        shellStartTestActivity()
        val resumed = instrumentation.waitForMonitorWithTimeout(secondMonitor, 10_000)
        assertNotNull("activity should return to foreground", resumed)
        resumed as TestLifecycleActivity
        instrumentation.waitForIdleSync()
        assertEquals(
            resumed.window.decorView.hasWindowFocus(),
            resumed.isClipboardListenerRegisteredForTesting(),
        )

        val recreateMonitor = instrumentation.addMonitor(
            TestLifecycleActivity::class.java.name,
            null,
            false,
        )
        instrumentation.runOnMainSync { resumed.recreate() }
        val recreated = instrumentation.waitForMonitorWithTimeout(recreateMonitor, 10_000)
        assertNotNull("recreate should produce a replacement activity", recreated)
        recreated as TestLifecycleActivity
        instrumentation.waitForIdleSync()
        assertEquals(
            recreated.window.decorView.hasWindowFocus(),
            recreated.isClipboardListenerRegisteredForTesting(),
        )
        assertSame(
            originalEngine,
            FlutterEngineCache.getInstance().get(MainActivity.ENGINE_ID),
        )
        instrumentation.runOnMainSync { recreated.finish() }

        instrumentation.removeMonitor(firstMonitor)
        instrumentation.removeMonitor(secondMonitor)
        instrumentation.removeMonitor(recreateMonitor)

        assertFalse(ClipboardListenerPolicy.shouldRegister(false, false))
        assertFalse(ClipboardListenerPolicy.shouldRegister(false, true))
        assertFalse(ClipboardListenerPolicy.shouldRegister(true, false))
        assertTrue(ClipboardListenerPolicy.shouldRegister(true, true))
    }

    private fun shellStartTestActivity() {
        val command = instrumentation.uiAutomation.executeShellCommand(
            "am start -W -n ${context.packageName}/.TestLifecycleActivity",
        )
        ParcelFileDescriptor.AutoCloseInputStream(command).bufferedReader().use {
            val output = it.readText()
            assertTrue("am start failed: $output", output.contains("Status: ok"))
        }
    }

}
