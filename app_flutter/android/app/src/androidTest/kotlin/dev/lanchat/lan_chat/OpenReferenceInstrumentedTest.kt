package dev.lanchat.lan_chat

import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class OpenReferenceInstrumentedTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val resolver = context.contentResolver
    private val treeUri: Uri = DocumentsContract.buildTreeDocumentUri(
        TestDocumentsProvider.AUTHORITY,
        TestDocumentsProvider.ROOT_ID,
    )

    @Test
    fun fileAndSavedReceiveDirectoryProduceResolvableViewIntents() {
        context.cacheDir.resolve("instrumented-documents").deleteRecursively()
        val root = DocumentsContract.buildDocumentUriUsingTree(
            treeUri,
            DocumentsContract.getTreeDocumentId(treeUri),
        )
        val file = requireNotNull(
            DocumentsContract.createDocument(resolver, root, "text/plain", "open-me.txt"),
        )

        val openFile = buildOpenReferenceIntent(
            context,
            file.toString(),
            showInFolder = false,
            receiveTreeReference = null,
        )
        assertEquals(Intent.ACTION_VIEW, openFile.action)
        assertEquals(file, openFile.data)
        assertEquals(resolver.getType(file), openFile.type)
        assertTrue(openFile.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)

        val showLocation = buildOpenReferenceIntent(
            context,
            file.toString(),
            showInFolder = true,
            receiveTreeReference = treeUri.toString(),
        )
        assertEquals(Intent.ACTION_OPEN_DOCUMENT_TREE, showLocation.action)
        assertEquals(
            treeUri,
            showLocation.getParcelableExtra(
                DocumentsContract.EXTRA_INITIAL_URI,
                Uri::class.java,
            ),
        )
        assertTrue(showLocation.flags and Intent.FLAG_ACTIVITY_NEW_DOCUMENT != 0)
    }
}
