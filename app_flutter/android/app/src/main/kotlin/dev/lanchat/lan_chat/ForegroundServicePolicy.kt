package dev.lanchat.lan_chat

object ForegroundServicePolicy {
    @JvmStatic
    fun shouldRun(
        keepOnline: Boolean,
        activeTransferCount: Int,
        stoppedByUser: Boolean,
        pausedBySystemTimeout: Boolean,
    ): Boolean =
        !stoppedByUser &&
            !pausedBySystemTimeout &&
            (keepOnline || activeTransferCount > 0)
}
