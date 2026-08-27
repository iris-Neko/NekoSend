package dev.lanchat.lan_chat

import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.provider.DocumentsContract
import android.system.ErrnoException
import android.system.OsConstants
import java.io.FileNotFoundException
import java.io.FileOutputStream
import java.security.MessageDigest

internal class SafReceiveAdapter(
    private val context: Context,
    private val preferencesName: String = MainActivity.PREFERENCES,
) {
    private val resolver = context.contentResolver
    private val preferences
        get() = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)

    fun prepareReceiveTree(
        treeUri: Uri,
        entries: List<Map<String, Any?>>,
    ): Map<String, Any> {
        require(entries.isNotEmpty()) { "传输清单为空" }
        probeReceiveTree(treeUri)
        val rootDocument = DocumentsContract.buildDocumentUriUsingTree(
            treeUri,
            DocumentsContract.getTreeDocumentId(treeUri),
        )
        val firstPath = requireNotNull(entries.first()["relativePath"] as? String)
        val rootName = firstPath.substringBefore('/')
        val isFolder = entries.first()["entryKind"] == "directory"
        val rootEntryId = requireNotNull(entries.first()["entryId"] as? String)
        val directoryByPath = mutableMapOf<String, Uri>()
        val prepared = mutableListOf<Map<String, Any?>>()
        val receiveRoot = if (isFolder) {
            loadPreparedUri(RECEIVE_ROOT_URI_PREFIX, rootEntryId, treeUri)
                ?.takeIf {
                    queryDocumentOrNull(it)?.mimeType == DocumentsContract.Document.MIME_TYPE_DIR
                }
                ?: run {
                    clearPreparedUri(RECEIVE_ROOT_URI_PREFIX, rootEntryId)
                    val uniqueRoot = uniqueChildName(rootDocument, rootName)
                    requireNotNull(
                        DocumentsContract.createDocument(
                            resolver,
                            rootDocument,
                            DocumentsContract.Document.MIME_TYPE_DIR,
                            uniqueRoot,
                        ),
                    ).also {
                        savePreparedUri(RECEIVE_ROOT_URI_PREFIX, rootEntryId, treeUri, it)
                    }
                }
        } else {
            rootDocument
        }.also {
            if (isFolder) directoryByPath[rootName] = it
        }

        for (entry in entries) {
            val entryId = requireNotNull(entry["entryId"] as? String)
            val entryKind = requireNotNull(entry["entryKind"] as? String)
            val relativePath = requireNotNull(entry["relativePath"] as? String)
            if (entryKind == "directory") {
                val directory = if (relativePath == rootName) {
                    receiveRoot
                } else {
                    val parentPath = relativePath.substringBeforeLast('/')
                    val parent = requireNotNull(directoryByPath[parentPath])
                    val name = relativePath.substringAfterLast('/')
                    findChild(parent, name)
                        ?.takeIf {
                            it.second.mimeType == DocumentsContract.Document.MIME_TYPE_DIR
                        }
                        ?.first
                        ?: requireNotNull(
                            DocumentsContract.createDocument(
                                resolver,
                                parent,
                                DocumentsContract.Document.MIME_TYPE_DIR,
                                name,
                            ),
                        )
                }
                directoryByPath[relativePath] = directory
                prepared += mapOf(
                    "entryId" to entryId,
                    "destinationRef" to directory.toString(),
                    "partialRef" to null,
                    "persistedOffset" to 0L,
                )
                continue
            }

            val parent = if (isFolder) {
                requireNotNull(directoryByPath[relativePath.substringBeforeLast('/')])
            } else {
                receiveRoot
            }
            val requestedName = relativePath.substringAfterLast('/')
            val partialName = ".lanchat-$entryId.partial"
            val expectedSize = (entry["size"] as Number).toLong()
            var partial = loadPreparedUri(RECEIVE_PARTIAL_URI_PREFIX, entryId, treeUri)
            var partialInfo = partial?.let(::queryDocumentOrNull)
            val savedFinalName = loadPreparedFinalName(entryId, treeUri)
            val reusableName = partialInfo?.name == partialName ||
                partialInfo?.name == "lanchat-$entryId.partial" ||
                (savedFinalName != null && partialInfo?.name == savedFinalName)
            if (!reusableName || (partialInfo.size ?: 0L) > expectedSize) {
                if (partial != null && reusableName) {
                    check(DocumentsContract.deleteDocument(resolver, partial)) {
                        "无法删除大小异常的接收临时文件"
                    }
                }
                clearPreparedUri(RECEIVE_PARTIAL_URI_PREFIX, entryId)
                partial = null
                partialInfo = null
            }

            val finalName = if (partial != null && savedFinalName != null) {
                savedFinalName
            } else {
                uniqueChildName(parent, requestedName).also {
                    savePreparedFinalName(entryId, treeUri, it)
                }
            }
            if (partial == null) {
                partial = findChild(parent, partialName)?.first
                    ?: findChild(parent, "lanchat-$entryId.partial")?.first
                    ?: DocumentsContract.createDocument(
                        resolver,
                        parent,
                        "application/octet-stream",
                        partialName,
                    )
                    ?: requireNotNull(
                        DocumentsContract.createDocument(
                            resolver,
                            parent,
                            "application/octet-stream",
                            "lanchat-$entryId.partial",
                        ),
                    )
                savePreparedUri(
                    RECEIVE_PARTIAL_URI_PREFIX,
                    entryId,
                    treeUri,
                    partial,
                )
                partialInfo = queryDocument(partial)
            }
            val existingSize = partialInfo?.size ?: 0L
            prepared += mapOf(
                "entryId" to entryId,
                "destinationRef" to finalName,
                "partialRef" to partial.toString(),
                "persistedOffset" to existingSize.coerceAtMost(expectedSize),
            )
        }
        return mapOf(
            "receiveBaseRef" to treeUri.toString(),
            "prepared" to prepared,
        )
    }

    fun probeReceiveTree(treeUri: Uri) {
        val root = DocumentsContract.buildDocumentUriUsingTree(
            treeUri,
            DocumentsContract.getTreeDocumentId(treeUri),
        )
        val probeName = ".lanchat-probe-${System.nanoTime()}"
        val created = requireNotNull(
            DocumentsContract.createDocument(
                resolver,
                root,
                "application/octet-stream",
                probeName,
            ),
        )
        var cleanupUri = created
        try {
            resolver.openFileDescriptor(created, "rw")!!.use { descriptor ->
                FileOutputStream(descriptor.fileDescriptor).use { output ->
                    output.write(1)
                    output.fd.sync()
                }
            }
            cleanupUri = requireNotNull(
                DocumentsContract.renameDocument(resolver, created, "$probeName-ok"),
            )
        } finally {
            check(DocumentsContract.deleteDocument(resolver, cleanupUri)) {
                "接收目录探针清理失败"
            }
        }
    }

    fun commitReceiveFile(requestedUri: Uri, finalName: String): Uri {
        val uri = resolveReceiveCommitUri(requestedUri)
        val renamed = if (queryDocumentOrNull(uri)?.name == finalName) {
            uri
        } else {
            DocumentsContract.renameDocument(
                resolver,
                uri,
                finalName,
            ) ?: error("文档提供器拒绝重命名接收文件")
        }
        saveReceiveCommitRedirect(requestedUri, uri, renamed)
        return renamed
    }

    private fun findChild(parent: Uri, name: String): Pair<Uri, DocumentInfo>? =
        queryChildren(parent, parent).firstOrNull { it.second.name == name }

    private fun queryDocumentOrNull(uri: Uri): DocumentInfo? = try {
        queryDocument(uri)
    } catch (_: SecurityException) {
        null
    } catch (_: FileNotFoundException) {
        null
    } catch (_: IllegalArgumentException) {
        null
    }

    private fun queryDocument(uri: Uri): DocumentInfo {
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )
        return resolver.query(uri, projection, null, null, null)?.use { cursor ->
            require(cursor.moveToFirst()) { "无法读取所选文档" }
            documentFromCursor(cursor, 0)
        } ?: error("文档提供器没有返回元数据")
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

    private fun documentFromCursor(cursor: Cursor, start: Int): DocumentInfo = DocumentInfo(
        name = cursor.getString(start),
        mimeType = cursor.getString(start + 1),
        size = if (cursor.isNull(start + 2)) null else cursor.getLong(start + 2),
        modifiedAtMs = if (cursor.isNull(start + 3)) 0L else cursor.getLong(start + 3),
    )

    private fun savePreparedUri(prefix: String, entryId: String, treeUri: Uri, uri: Uri) {
        check(
            preferences.edit()
                .putString("$prefix$entryId", uri.toString())
                .putString("$RECEIVE_TREE_URI_PREFIX$entryId", treeUri.toString())
                .commit(),
        ) { "无法保存接收文件恢复信息" }
    }

    private fun loadPreparedUri(prefix: String, entryId: String, treeUri: Uri): Uri? {
        if (preferences.getString("$RECEIVE_TREE_URI_PREFIX$entryId", null) != treeUri.toString()) {
            clearPreparedUri(prefix, entryId)
            return null
        }
        return preferences.getString("$prefix$entryId", null)?.let(Uri::parse)
    }

    private fun clearPreparedUri(prefix: String, entryId: String) {
        preferences.edit().remove("$prefix$entryId").apply()
    }

    private fun savePreparedFinalName(entryId: String, treeUri: Uri, name: String) {
        check(
            preferences.edit()
                .putString("$RECEIVE_FINAL_NAME_PREFIX$entryId", name)
                .putString("$RECEIVE_TREE_URI_PREFIX$entryId", treeUri.toString())
                .commit(),
        ) { "无法保存接收文件名" }
    }

    private fun loadPreparedFinalName(entryId: String, treeUri: Uri): String? {
        if (preferences.getString("$RECEIVE_TREE_URI_PREFIX$entryId", null) != treeUri.toString()) {
            return null
        }
        return preferences.getString("$RECEIVE_FINAL_NAME_PREFIX$entryId", null)
    }

    private fun resolveReceiveCommitUri(requestedUri: Uri): Uri {
        var resolved = requestedUri
        repeat(4) {
            val redirected = preferences.getString(receiveRedirectKey(resolved), null)
                ?: return resolved
            val next = Uri.parse(redirected)
            if (next == resolved) return resolved
            resolved = next
        }
        return resolved
    }

    private fun saveReceiveCommitRedirect(requestedUri: Uri, sourceUri: Uri, renamedUri: Uri) {
        val editor = preferences.edit()
            .putString(receiveRedirectKey(requestedUri), renamedUri.toString())
            .putString(receiveRedirectKey(sourceUri), renamedUri.toString())
        for ((key, value) in preferences.all) {
            if (key.startsWith(RECEIVE_PARTIAL_URI_PREFIX) &&
                (value == requestedUri.toString() || value == sourceUri.toString())
            ) {
                editor.putString(key, renamedUri.toString())
            }
        }
        check(editor.commit()) { "无法保存接收文件提交结果" }
    }

    private fun receiveRedirectKey(uri: Uri): String {
        val digest = MessageDigest.getInstance("SHA-256")
            .digest(uri.toString().toByteArray(Charsets.UTF_8))
            .joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }
        return "$RECEIVE_REDIRECT_URI_PREFIX$digest"
    }

    private fun uniqueChildName(parent: Uri, requested: String): String {
        val existing = queryChildren(parent, parent).mapTo(mutableSetOf()) { it.second.name }
        if (requested !in existing) return requested
        val dot = requested.lastIndexOf('.')
        val hasExtension = dot > 0 && dot < requested.lastIndex
        val stem = if (hasExtension) requested.substring(0, dot) else requested
        val extension = if (hasExtension) requested.substring(dot) else ""
        var sequence = 1
        while (true) {
            val candidate = "$stem ($sequence)$extension"
            if (candidate !in existing) return candidate
            sequence++
        }
    }

    private data class DocumentInfo(
        val name: String,
        val mimeType: String,
        val size: Long?,
        val modifiedAtMs: Long,
    )

    companion object {
        private const val RECEIVE_ROOT_URI_PREFIX = "receive_root_uri_"
        private const val RECEIVE_PARTIAL_URI_PREFIX = "receive_partial_uri_"
        private const val RECEIVE_FINAL_NAME_PREFIX = "receive_final_name_"
        private const val RECEIVE_TREE_URI_PREFIX = "receive_tree_uri_"
        private const val RECEIVE_REDIRECT_URI_PREFIX = "receive_redirect_uri_"
    }
}

internal fun androidPlatformErrorCode(error: Throwable): String {
    var current: Throwable? = error
    while (current != null) {
        val message = current.message.orEmpty()
        if (current is SecurityException ||
            current is FileNotFoundException ||
            message.contains("SecurityException", ignoreCase = true) ||
            message.contains("Permission Denial", ignoreCase = true) ||
            message.contains("access revoked", ignoreCase = true)
        ) {
            return "FILE_PERMISSION_LOST"
        }
        if (current is ErrnoException &&
            (current.errno == OsConstants.ENOSPC || current.errno == OsConstants.EDQUOT)
        ) {
            return "FILE_NO_SPACE"
        }
        current = current.cause
    }
    return "ANDROID_PLATFORM_IO_FAILED"
}
