package dev.lanchat.lan_chat

import android.database.Cursor
import android.database.MatrixCursor
import android.content.Context
import android.os.CancellationSignal
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelFileDescriptor
import android.os.ProxyFileDescriptorCallback
import android.os.storage.StorageManager
import android.provider.DocumentsContract
import android.provider.DocumentsProvider
import android.system.ErrnoException
import android.system.OsConstants
import android.webkit.MimeTypeMap
import java.io.File
import java.io.FileNotFoundException
import java.io.RandomAccessFile
import java.net.URLDecoder
import java.net.URLEncoder

class TestDocumentsProvider : DocumentsProvider() {
    private val proxyHandler: Handler by lazy {
        HandlerThread("lan-chat-test-documents").also { it.start() }.let {
            Handler(it.looper)
        }
    }

    private val root: File
        get() = File(requireNotNull(context).cacheDir, "instrumented-documents").apply {
            mkdirs()
        }

    override fun onCreate(): Boolean = true

    override fun queryRoots(projection: Array<out String>?): Cursor {
        val columns = projection ?: DEFAULT_ROOT_PROJECTION
        return MatrixCursor(columns).apply {
            val row = newRow()
            put(row, columns, DocumentsContract.Root.COLUMN_ROOT_ID, ROOT_ID)
            put(row, columns, DocumentsContract.Root.COLUMN_DOCUMENT_ID, ROOT_ID)
            put(row, columns, DocumentsContract.Root.COLUMN_TITLE, "LAN Chat test provider")
            put(row, columns, DocumentsContract.Root.COLUMN_FLAGS,
                DocumentsContract.Root.FLAG_SUPPORTS_CREATE)
        }
    }

    override fun queryDocument(documentId: String, projection: Array<out String>?): Cursor =
        MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION).apply {
            checkAccess()
            addDocument(newRow(), columnNames, fileForId(documentId))
        }

    override fun queryChildDocuments(
        parentDocumentId: String,
        projection: Array<out String>?,
        sortOrder: String?,
    ): Cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION).apply {
        checkAccess()
        val parent = fileForId(parentDocumentId)
        parent.listFiles()?.sortedBy { it.name }?.forEach {
            addDocument(newRow(), columnNames, it)
        }
    }

    override fun openDocument(
        documentId: String,
        mode: String,
        signal: CancellationSignal?,
    ): ParcelFileDescriptor {
        checkAccess()
        val file = fileForId(documentId)
        if (mode.contains('w') && file.name.endsWith(".partial") &&
            failureEnabled(FAIL_SPACE)
        ) {
            val storage = requireNotNull(context).getSystemService(StorageManager::class.java)
            val randomAccess = RandomAccessFile(file, "rw")
            return storage.openProxyFileDescriptor(
                ParcelFileDescriptor.parseMode(mode),
                LimitedFileCallback(randomAccess),
                proxyHandler,
            )
        }
        return ParcelFileDescriptor.open(
            file,
            ParcelFileDescriptor.parseMode(mode),
        )
    }

    override fun createDocument(
        parentDocumentId: String,
        mimeType: String,
        displayName: String,
    ): String {
        checkAccess()
        val parent = fileForId(parentDocumentId)
        val child = File(parent, displayName)
        val created = if (mimeType == DocumentsContract.Document.MIME_TYPE_DIR) {
            child.mkdir()
        } else {
            child.createNewFile()
        }
        if (!created) throw FileNotFoundException("cannot create $displayName")
        return idForFile(child)
    }

    override fun renameDocument(documentId: String, displayName: String): String {
        checkAccess()
        val source = fileForId(documentId)
        val target = File(requireNotNull(source.parentFile), displayName)
        if (failureEnabled(FAIL_RENAME) || !source.renameTo(target)) {
            throw FileNotFoundException("rename rejected")
        }
        return idForFile(target)
    }

    override fun deleteDocument(documentId: String) {
        checkAccess()
        if (failureEnabled(FAIL_DELETE) || !fileForId(documentId).deleteRecursively()) {
            throw FileNotFoundException("delete rejected")
        }
    }

    override fun isChildDocument(parentDocumentId: String, documentId: String): Boolean {
        checkAccess()
        val parent = fileForId(parentDocumentId).canonicalFile.toPath()
        val child = fileForId(documentId).canonicalFile.toPath()
        return child.startsWith(parent) && child != parent
    }

    private fun addDocument(
        row: MatrixCursor.RowBuilder,
        columns: Array<out String>,
        file: File,
    ) {
        val flags = if (file.isDirectory) {
            DocumentsContract.Document.FLAG_DIR_SUPPORTS_CREATE or
                DocumentsContract.Document.FLAG_SUPPORTS_DELETE or
                DocumentsContract.Document.FLAG_SUPPORTS_RENAME
        } else {
            DocumentsContract.Document.FLAG_SUPPORTS_WRITE or
                DocumentsContract.Document.FLAG_SUPPORTS_DELETE or
                DocumentsContract.Document.FLAG_SUPPORTS_RENAME
        }
        put(row, columns, DocumentsContract.Document.COLUMN_DOCUMENT_ID, idForFile(file))
        put(row, columns, DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            if (file == root) "root" else file.name)
        put(row, columns, DocumentsContract.Document.COLUMN_MIME_TYPE,
            if (file.isDirectory) DocumentsContract.Document.MIME_TYPE_DIR else
                MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream")
        put(row, columns, DocumentsContract.Document.COLUMN_FLAGS, flags)
        put(row, columns, DocumentsContract.Document.COLUMN_SIZE,
            if (file.isDirectory) null else file.length())
        put(row, columns, DocumentsContract.Document.COLUMN_LAST_MODIFIED, file.lastModified())
    }

    private fun fileForId(documentId: String): File {
        if (documentId == ROOT_ID) return root
        if (!documentId.startsWith(DOCUMENT_PREFIX)) throw FileNotFoundException("bad document id")
        val relative = URLDecoder.decode(documentId.removePrefix(DOCUMENT_PREFIX), Charsets.UTF_8.name())
        val file = File(root, relative).canonicalFile
        if (!file.toPath().startsWith(root.canonicalFile.toPath())) {
            throw FileNotFoundException("document escaped root")
        }
        if (!file.exists()) throw FileNotFoundException("document missing")
        return file
    }

    private fun checkAccess() {
        if (failureEnabled(FAIL_ACCESS)) {
            throw SecurityException("test provider access revoked")
        }
    }

    private fun failureEnabled(name: String): Boolean =
        File(requireNotNull(context).cacheDir, name).exists()

    private fun idForFile(file: File): String {
        if (file.canonicalFile == root.canonicalFile) return ROOT_ID
        val relative = root.canonicalFile.toPath().relativize(file.canonicalFile.toPath()).toString()
        return DOCUMENT_PREFIX + URLEncoder.encode(relative, Charsets.UTF_8.name())
    }

    companion object {
        const val AUTHORITY = "dev.lanchat.lan_chat.test.documents"
        const val ROOT_ID = "root"
        private const val DOCUMENT_PREFIX = "doc:"
        private const val FAIL_RENAME = "fail-rename"
        private const val FAIL_DELETE = "fail-delete"
        private const val FAIL_ACCESS = "fail-access"
        private const val FAIL_SPACE = "fail-space"
        private const val STORAGE_LIMIT_BYTES = 1024L * 1024L

        fun setFailure(context: Context, name: String, enabled: Boolean) {
            val fileName = when (name) {
                "rename" -> FAIL_RENAME
                "delete" -> FAIL_DELETE
                "access" -> FAIL_ACCESS
                "space" -> FAIL_SPACE
                else -> error("unknown provider failure: $name")
            }
            val marker = File(context.cacheDir, fileName)
            if (enabled) {
                check(marker.createNewFile() || marker.isFile)
            } else {
                marker.delete()
            }
        }

        private val DEFAULT_ROOT_PROJECTION = arrayOf(
            DocumentsContract.Root.COLUMN_ROOT_ID,
            DocumentsContract.Root.COLUMN_DOCUMENT_ID,
            DocumentsContract.Root.COLUMN_TITLE,
            DocumentsContract.Root.COLUMN_FLAGS,
        )
        private val DEFAULT_DOCUMENT_PROJECTION = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_FLAGS,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )

        private fun put(
            row: MatrixCursor.RowBuilder,
            columns: Array<out String>,
            column: String,
            value: Any?,
        ) {
            if (column in columns) row.add(column, value)
        }
    }

    private class LimitedFileCallback(
        private val file: RandomAccessFile,
    ) : ProxyFileDescriptorCallback() {
        override fun onGetSize(): Long = synchronized(file) { file.length() }

        override fun onRead(offset: Long, size: Int, data: ByteArray): Int = synchronized(file) {
            file.seek(offset)
            file.read(data, 0, size).coerceAtLeast(0)
        }

        override fun onWrite(offset: Long, size: Int, data: ByteArray): Int = synchronized(file) {
            if (offset >= STORAGE_LIMIT_BYTES || size.toLong() > STORAGE_LIMIT_BYTES - offset) {
                throw ErrnoException("write", OsConstants.ENOSPC)
            }
            file.seek(offset)
            file.write(data, 0, size)
            size
        }

        override fun onFsync() = synchronized(file) {
            file.fd.sync()
        }

        override fun onRelease() = synchronized(file) {
            file.close()
        }
    }
}
