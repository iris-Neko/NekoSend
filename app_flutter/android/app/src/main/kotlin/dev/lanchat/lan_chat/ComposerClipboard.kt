package dev.lanchat.lan_chat

import android.content.ClipboardManager
import android.content.ClipData
import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.ConcurrentHashMap

internal class ComposerClipboard(private val context: Context) {
    companion object {
        @Volatile private var instance: ComposerClipboard? = null
        // Keep cancellation state across Activity recreation, without retaining an Activity.
        fun shared(context: Context): ComposerClipboard = instance ?: synchronized(this) {
            instance ?: ComposerClipboard(context.applicationContext).also { instance = it }
        }
    }
    private val cancelled = ConcurrentHashMap.newKeySet<String>()
    private val active = ConcurrentHashMap.newKeySet<String>()
    private val resolver get() = context.contentResolver

    fun read(clip: ClipData?): Map<String, Any> {
        if (clip == null) return emptyMap()
        if (clip.itemCount == 0) return emptyMap()
        require(clip.itemCount <= 10_000) { "剪贴板文件数量过多" }
        val files = (0 until clip.itemCount).mapNotNull { index ->
            val uri = clip.getItemAt(index).uri ?: return@mapNotNull null
            if (uri.scheme != "content") return@mapNotNull null
            val mime = resolver.getType(documentUri(uri)) ?: "application/octet-stream"
            mapOf("sourceRef" to uri.toString(), "displayName" to name(uri),
                "kind" to when { mime == DocumentsContract.Document.MIME_TYPE_DIR -> "folder"
                    mime.startsWith("image/") -> "image"; else -> "file" },
                "ephemeral" to true, "fingerprint" to "${clip.description.timestamp}:$index:$uri")
        }
        if (files.isNotEmpty()) return mapOf("items" to files)
        return mapOf("text" to (clip.getItemAt(0).text?.toString() ?: clip.getItemAt(0).uri?.toString() ?: ""))
    }

    fun register(token: String) { active.add(token) }
    fun cancel(token: String) { if (active.contains(token)) cancelled.add(token) }

    fun prepare(uri: Uri, kind: String, token: String, session: String,
                progress: (Long, Long?) -> Unit): Map<String, String> {
        require(token.matches(Regex("[a-zA-Z0-9-]{1,80}")) && session.matches(Regex("[a-zA-Z0-9-]{1,80}")))
        require(uri.scheme == "content")
        val root = File(context.filesDir, "composer-sources").canonicalFile
        val owned = File(root, "$session/$token").canonicalFile
        require(owned.toPath().startsWith(root.toPath()))
        val payload = File(owned, "payload").apply { mkdirs() }.canonicalFile
        fun destination(relative: String): File {
            require(relative.isNotBlank() && !relative.contains('\\') &&
                relative.split('/').none { it.isEmpty() || it == "." || it == ".." }) { "附件名称无效" }
            val file = File(payload, relative).canonicalFile
            require(file.toPath().startsWith(payload.toPath()) && file != payload) { "附件路径无效" }
            return file
        }
        var bytes = 0L
        var lastProgress = 0L
        fun check() { kotlin.check(!cancelled.contains(token)) { "附件已移除" } }
        fun copy(source: Uri, file: File, total: Long?) {
            check()
            file.parentFile?.mkdirs()
            resolver.openInputStream(source).use { input ->
                requireNotNull(input) { "文件不可读取，请使用附件选择器" }
                FileOutputStream(file).use { output ->
                    val buffer = ByteArray(256 * 1024)
                    while (true) {
                        check()
                        val count = input.read(buffer)
                        if (count < 0) break
                        output.write(buffer, 0, count)
                        bytes += count
                        val now = System.nanoTime()
                        if (now - lastProgress >= 100_000_000L) { progress(bytes, total); lastProgress = now }
                    }
                    output.fd.sync()
                }
            }
        }
        try {
            check()
            val displayName: String
            if (kind == "folder") {
                val picked = SafSourceAdapter(context).enumerate(uri, kind)
                displayName = picked["displayName"] as String
                @Suppress("UNCHECKED_CAST")
                val entries = picked["sources"] as List<Map<String, Any?>>
                val total = entries.sumOf { (it["size"] as Number).toLong() }
                require(payload.usableSpace > total) { "存储空间不足" }
                progress(0, total)
                for (entry in entries) {
                    check()
                    val file = destination(entry["relativePath"] as String)
                    if (entry["entryKind"] == "directory") file.mkdirs()
                    else copy(Uri.parse(entry["sourceRef"] as String), file, total)
                }
            } else {
                displayName = name(uri)
                progress(0, null)
                copy(uri, destination(displayName), null)
            }
            check()
            progress(bytes, bytes)
            return mapOf("path" to destination(displayName).absolutePath, "ownedDirectory" to owned.absolutePath)
        } catch (error: Throwable) {
            owned.deleteRecursively()
            throw error
        } finally { cancelled.remove(token); active.remove(token) }
    }

    private fun documentUri(uri: Uri): Uri {
        if (!DocumentsContract.isTreeUri(uri)) return uri
        // isDocumentUri also checks provider registration; a valid tree/document
        // reference must never be broadened to the grant's root based on that.
        val documentId = runCatching { DocumentsContract.getDocumentId(uri) }.getOrNull()
        return if (documentId != null) uri else
            DocumentsContract.buildDocumentUriUsingTree(uri, DocumentsContract.getTreeDocumentId(uri))
    }

    private fun name(uri: Uri): String = resolver.query(documentUri(uri), arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
        if (it.moveToFirst()) it.getString(0) else null
    }?.takeIf { it.isNotBlank() } ?: "attachment"
}
