package dev.lanchat.lan_chat

import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.FileOutputStream

@RunWith(AndroidJUnit4::class)
class DocumentFdInstrumentedTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val treeUri: Uri = DocumentsContract.buildTreeDocumentUri(
        TestDocumentsProvider.AUTHORITY,
        TestDocumentsProvider.ROOT_ID,
    )

    @Before
    fun setUp() {
        context.cacheDir.resolve("instrumented-documents").deleteRecursively()
        TestDocumentsProvider.setFailure(context, "access", false)
        TestDocumentsProvider.setFailure(context, "rename", false)
        TestDocumentsProvider.setFailure(context, "delete", false)
    }

    @Test
    fun detachedWritableFdIsClosedExactlyOnceByFailureCleanup() {
        val root = DocumentsContract.buildDocumentUriUsingTree(
            treeUri,
            DocumentsContract.getTreeDocumentId(treeUri),
        )
        val document = requireNotNull(
            DocumentsContract.createDocument(
                context.contentResolver,
                root,
                "application/octet-stream",
                "fd-test.bin",
            ),
        )
        val adapter = DocumentFdAdapter(context)
        val detached = adapter.open(document, writable = true)
        val adopted = ParcelFileDescriptor.adoptFd(detached)
        val descriptor = adopted.fileDescriptor
        FileOutputStream(descriptor).write(byteArrayOf(9, 8, 7, 6))
        val returned = adopted.detachFd()
        assertEquals(detached, returned)

        adapter.close(returned)

        try {
            Os.fstat(descriptor)
            fail("closed descriptor unexpectedly remained valid")
        } catch (error: ErrnoException) {
            assertEquals(OsConstants.EBADF, error.errno)
        }
    }
}
