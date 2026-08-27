package dev.lanchat.lan_chat

import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SafSourceInstrumentedTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val resolver = context.contentResolver
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
    fun fileUriCanBeEnumeratedAndOpenedAfterAdapterRecreation() {
        val root = rootDocument(treeUri)
        val file = requireNotNull(
            DocumentsContract.createDocument(
                resolver,
                root,
                "application/octet-stream",
                "source.bin",
            ),
        )
        val payload = byteArrayOf(1, 3, 5, 7, 9)
        resolver.openOutputStream(file, "w")!!.use { it.write(payload) }

        val first = sources(SafSourceAdapter(context).enumerate(file, "file")).single()
        val recreated = sources(SafSourceAdapter(context).enumerate(file, "file")).single()

        assertEquals(file.toString(), first["sourceRef"])
        assertEquals(first, recreated)
        assertEquals(payload.size.toLong(), (recreated["size"] as Number).toLong())

        val rawFd = DocumentFdAdapter(context).open(file, writable = false)
        val bytes = ParcelFileDescriptor.AutoCloseInputStream(
            ParcelFileDescriptor.adoptFd(rawFd),
        ).use { it.readBytes() }
        assertArrayEquals(payload, bytes)
    }

    @Test
    fun folderManifestPreservesRootEmptyDirectoryAndFile() {
        val providerRoot = rootDocument(treeUri)
        val selectedFolder = requireNotNull(
            DocumentsContract.createDocument(
                resolver,
                providerRoot,
                DocumentsContract.Document.MIME_TYPE_DIR,
                "send-root",
            ),
        )
        requireNotNull(
            DocumentsContract.createDocument(
                resolver,
                selectedFolder,
                DocumentsContract.Document.MIME_TYPE_DIR,
                "empty",
            ),
        )
        val file = requireNotNull(
            DocumentsContract.createDocument(
                resolver,
                selectedFolder,
                "application/octet-stream",
                "data.bin",
            ),
        )
        resolver.openOutputStream(file, "w")!!.use { it.write(byteArrayOf(2, 4, 6)) }
        val selectedTree = DocumentsContract.buildTreeDocumentUri(
            TestDocumentsProvider.AUTHORITY,
            DocumentsContract.getDocumentId(selectedFolder),
        )

        val manifest = sources(SafSourceAdapter(context).enumerate(selectedTree, "folder"))
        val byPath = manifest.associateBy { it.getValue("relativePath") as String }

        assertEquals(setOf("send-root", "send-root/empty", "send-root/data.bin"), byPath.keys)
        assertEquals("directory", byPath.getValue("send-root")["entryKind"])
        assertEquals(
            rootDocument(selectedTree).toString(),
            byPath.getValue("send-root")["sourceRef"],
        )
        assertEquals("directory", byPath.getValue("send-root/empty")["entryKind"])
        assertEquals(null, byPath.getValue("send-root/empty")["sourceRef"])
        assertEquals("file", byPath.getValue("send-root/data.bin")["entryKind"])
        assertEquals(3L, (byPath.getValue("send-root/data.bin")["size"] as Number).toLong())
    }

    private fun rootDocument(uri: Uri): Uri = DocumentsContract.buildDocumentUriUsingTree(
        uri,
        DocumentsContract.getTreeDocumentId(uri),
    )

    private fun sources(result: Map<String, Any>): List<Map<String, Any?>> {
        @Suppress("UNCHECKED_CAST")
        return result.getValue("sources") as List<Map<String, Any?>>
    }
}
