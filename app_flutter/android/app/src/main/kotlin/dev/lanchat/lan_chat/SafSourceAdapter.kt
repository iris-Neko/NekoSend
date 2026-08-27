package dev.lanchat.lan_chat

import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns

internal class SafSourceAdapter(context: Context) {
    private val resolver = context.contentResolver

    fun enumerate(uri: Uri, kind: String): Map<String, Any> {
        if (kind != "folder") {
            val document = queryDocument(uri)
            require(document.size != null) { "所选文档没有可用的文件大小" }
            return mapOf(
                "displayName" to document.name,
                "sources" to listOf(
                    mapOf(
                        "entryKind" to "file",
                        "sourceRef" to uri.toString(),
                        "relativePath" to document.name,
                        "size" to document.size,
                        "modifiedAtMs" to document.modifiedAtMs,
                    ),
                ),
            )
        }

        val rootId = DocumentsContract.getTreeDocumentId(uri)
        val rootUri = DocumentsContract.buildDocumentUriUsingTree(uri, rootId)
        val root = queryDocument(rootUri)
        val sources = mutableListOf<Map<String, Any?>>()
        sources += mapOf(
            "entryKind" to "directory",
            "sourceRef" to rootUri.toString(),
            "relativePath" to root.name,
            "size" to 0L,
            "modifiedAtMs" to root.modifiedAtMs,
        )
        var fileCount = 0
        val queue = ArrayDeque<Pair<Uri, String>>()
        queue.add(rootUri to root.name)
        while (queue.isNotEmpty()) {
            val (directory, relativeDirectory) = queue.removeFirst()
            for ((childUri, child) in queryChildren(uri, directory)) {
                require(sources.size < MAX_MANIFEST_ENTRIES) {
                    "文件夹项目总数超过 20001"
                }
                val relativePath = "$relativeDirectory/${child.name}"
                if (child.mimeType == DocumentsContract.Document.MIME_TYPE_DIR) {
                    sources += mapOf(
                        "entryKind" to "directory",
                        "sourceRef" to null,
                        "relativePath" to relativePath,
                        "size" to 0L,
                        "modifiedAtMs" to child.modifiedAtMs,
                    )
                    queue.add(childUri to relativePath)
                } else {
                    require(fileCount < MAX_FILE_ENTRIES) {
                        "文件夹文件数超过 10000"
                    }
                    require(child.size != null) {
                        "文档 ${child.name} 没有可用的文件大小"
                    }
                    sources += mapOf(
                        "entryKind" to "file",
                        "sourceRef" to childUri.toString(),
                        "relativePath" to relativePath,
                        "size" to child.size,
                        "modifiedAtMs" to child.modifiedAtMs,
                    )
                    fileCount += 1
                }
            }
        }
        return mapOf("displayName" to root.name, "sources" to sources)
    }

    private fun queryChildren(treeUri: Uri, parent: Uri): List<Pair<Uri, DocumentInfo>> {
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(
            treeUri,
            DocumentsContract.getDocumentId(parent),
        )
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )
        return resolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
            buildList {
                while (cursor.moveToNext()) {
                    val documentId = cursor.getString(0)
                    add(
                        DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId) to
                            documentFromCursor(cursor, 1),
                    )
                }
            }
        } ?: emptyList()
    }

    private fun queryDocument(uri: Uri): DocumentInfo {
        val projection = arrayOf(
            OpenableColumns.DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            OpenableColumns.SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )
        return resolver.query(uri, projection, null, null, null)?.use { cursor ->
            require(cursor.moveToFirst()) { "无法读取所选文档" }
            documentFromCursor(cursor, 0)
        } ?: error("文档提供器没有返回元数据")
    }

    private fun documentFromCursor(cursor: Cursor, start: Int): DocumentInfo = DocumentInfo(
        name = cursor.getString(start),
        mimeType = cursor.getString(start + 1),
        size = if (cursor.isNull(start + 2)) null else cursor.getLong(start + 2),
        modifiedAtMs = if (cursor.isNull(start + 3)) 0L else cursor.getLong(start + 3),
    )

    private data class DocumentInfo(
        val name: String,
        val mimeType: String,
        val size: Long?,
        val modifiedAtMs: Long,
    )

    companion object {
        private const val MAX_FILE_ENTRIES = 10_000
        private const val MAX_MANIFEST_ENTRIES = 20_001
    }
}
