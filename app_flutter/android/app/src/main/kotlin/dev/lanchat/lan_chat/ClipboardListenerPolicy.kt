package dev.lanchat.lan_chat

object ClipboardListenerPolicy {
    @JvmStatic
    fun shouldRegister(activityResumed: Boolean, hasWindowFocus: Boolean): Boolean =
        activityResumed && hasWindowFocus
}
