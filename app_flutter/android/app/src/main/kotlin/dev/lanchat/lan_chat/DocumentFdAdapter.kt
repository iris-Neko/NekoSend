package dev.lanchat.lan_chat

import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor

internal class DocumentFdAdapter(private val context: Context) {
    fun open(uri: Uri, writable: Boolean): Int {
        val descriptor = context.contentResolver.openFileDescriptor(
            uri,
            if (writable) "rw" else "r",
        ) ?: error("文档提供器没有返回文件描述符")
        return descriptor.detachFd()
    }

    fun close(fd: Int) {
        require(fd >= 0) { "文件描述符无效" }
        ParcelFileDescriptor.adoptFd(fd).close()
    }
}
