package dev.lanchat.lan_chat;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class ForegroundServicePolicyTest {
    @Test
    public void keepOnlineOrActiveTransferStartsService() {
        assertTrue(ForegroundServicePolicy.shouldRun(true, 0, false, false));
        assertTrue(ForegroundServicePolicy.shouldRun(false, 1, false, false));
        assertFalse(ForegroundServicePolicy.shouldRun(false, 0, false, false));
    }

    @Test
    public void explicitStopAndSystemTimeoutSuppressAutomaticRestart() {
        assertFalse(ForegroundServicePolicy.shouldRun(true, 2, true, false));
        assertFalse(ForegroundServicePolicy.shouldRun(true, 2, false, true));
    }
}
