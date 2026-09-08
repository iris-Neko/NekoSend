package dev.lanchat.lan_chat

internal object DeviceNamePolicy {
    @JvmStatic
    fun chooseDefaultName(
        model: String,
        marketNames: List<String>,
        systemDeviceName: String?,
    ): String = (marketNames + listOfNotNull(systemDeviceName) + model)
        .map(String::trim)
        .firstOrNull {
            it.isNotEmpty() &&
                !it.equals("unknown", ignoreCase = true) &&
                !it.equals("null", ignoreCase = true) &&
                it.none(Char::isISOControl)
        } ?: "Android"
}
