package dev.lanchat.lan_chat

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract

internal fun buildOpenReferenceIntent(
    context: Context,
    reference: String,
    showInFolder: Boolean,
    receiveTreeReference: String?,
): Intent {
    val requested = Uri.parse(reference)
    require(requested.scheme == "content") { "Android 文件必须来自 SAF" }
    val uri = if (showInFolder) {
        receiveTreeReference?.let(Uri::parse) ?: requested
    } else {
        requested
    }
    val mime = context.contentResolver.getType(uri)
    if (showInFolder || mime == DocumentsContract.Document.MIME_TYPE_DIR) {
        return Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            putExtra(DocumentsContract.EXTRA_INITIAL_URI, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_DOCUMENT)
        }
    }
    return Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uri, mime ?: "*/*")
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        addFlags(Intent.FLAG_ACTIVITY_NEW_DOCUMENT)
    }
}
