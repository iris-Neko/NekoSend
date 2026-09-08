package dev.lanchat.lan_chat;

import static org.junit.Assert.assertEquals;

import java.util.List;
import org.junit.Test;

public final class DeviceNamePolicyTest {
    @Test
    public void marketNameIsPreferredToModelCodeAndSystemNickname() {
        assertEquals("Xiaomi 14", DeviceNamePolicy.chooseDefaultName(
            "23127PN0CC", List.of(" Xiaomi 14 ", "Other model"), "My phone"));
    }

    @Test
    public void systemNameIsUsedWhenMarketingPropertiesAreMissing() {
        assertEquals("Galaxy S24", DeviceNamePolicy.chooseDefaultName(
            "SM-S921B", List.of("", "unknown", "null"), "Galaxy S24"));
    }

    @Test
    public void buildModelRemainsTheFallback() {
        assertEquals("Pixel 8", DeviceNamePolicy.chooseDefaultName(
            "Pixel 8", List.of(), null));
    }

    @Test
    public void allMissingValuesUseANonEmptyFallback() {
        assertEquals("Android", DeviceNamePolicy.chooseDefaultName(
            "unknown", List.of("", "NULL"), " "));
    }

    @Test
    public void invalidPropertyDoesNotHideAValidFallback() {
        assertEquals("Mi 11i", DeviceNamePolicy.chooseDefaultName(
            "M2012K11G", List.of("bad\u0000name", "Mi 11i"), null));
    }
}
