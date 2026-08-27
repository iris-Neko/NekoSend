package dev.lanchat.lan_chat

import android.os.Bundle

class TestLifecycleActivity : MainActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        setShowWhenLocked(true)
        setTurnScreenOn(true)
        super.onCreate(savedInstanceState)
    }
}
