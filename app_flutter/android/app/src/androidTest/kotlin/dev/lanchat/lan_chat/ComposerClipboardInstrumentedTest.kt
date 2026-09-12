package dev.lanchat.lan_chat

import android.content.ClipData
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.DocumentsContract
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class ComposerClipboardInstrumentedTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val resolver = context.contentResolver
    private val bridge = ComposerClipboard(context)
    private val session = "composer-instrumentation"
    private val sourceBytes = byteArrayOf(1, 3, 5, 7, 9)
    private lateinit var source: Uri
    private val additional = mutableListOf<Uri>()

    @Before fun setUp() {
        TestDocumentsProvider.setFailure(context, "access", false)
        val tree = DocumentsContract.buildTreeDocumentUri(TestDocumentsProvider.AUTHORITY, TestDocumentsProvider.ROOT_ID)
        val root = DocumentsContract.buildDocumentUriUsingTree(tree, TestDocumentsProvider.ROOT_ID)
        source = requireNotNull(DocumentsContract.createDocument(resolver, root, "application/octet-stream", "composer-source.bin"))
        resolver.openOutputStream(source, "w")!!.use { it.write(sourceBytes) }
    }

    @After fun tearDown() {
        TestDocumentsProvider.setFailure(context, "access", false)
        DocumentsContract.deleteDocument(resolver, source)
        additional.forEach { DocumentsContract.deleteDocument(resolver, it) }
        val base = File(context.filesDir, "composer-sources").canonicalFile
        val owned = File(base, session).canonicalFile
        require(owned.toPath().startsWith(base.toPath()) && owned != base)
        owned.deleteRecursively()
    }

    @Test fun fileUrisBecomeStableSourcesWithoutChangingTheOriginal() {
        val clip = ClipData.newRawUri("files", source)
        clip.addItem(ClipData.Item(source))
        val content = bridge.read(clip)
        assertEquals(2, (content["items"] as List<*>).size)
        bridge.register("copy")
        val prepared = bridge.prepare(source, "file", "copy", session) { _, _ -> }
        assertArrayEquals(sourceBytes, File(prepared.getValue("path")).readBytes())
        assertArrayEquals(sourceBytes, resolver.openInputStream(source)!!.use { it.readBytes() })
    }

    @Test fun plainPathsRemainText() {
        val text = "/storage/emulated/0/private-file.txt"
        val content = bridge.read(ClipData.newPlainText("text", text))
        assertEquals(text, content["text"])
        assertFalse(content.containsKey("items"))
    }

    @Test fun imageUrisKeepImageContent() {
        val tree = DocumentsContract.buildTreeDocumentUri(TestDocumentsProvider.AUTHORITY, TestDocumentsProvider.ROOT_ID)
        val root = DocumentsContract.buildDocumentUriUsingTree(tree, TestDocumentsProvider.ROOT_ID)
        val image = requireNotNull(DocumentsContract.createDocument(resolver, root, "image/png", "clipboard-image.png"))
        additional.add(image)
        val bitmap = Bitmap.createBitmap(2, 2, Bitmap.Config.ARGB_8888)
        resolver.openOutputStream(image, "w")!!.use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        val items = bridge.read(ClipData.newRawUri("image", image))["items"] as List<*>
        assertEquals("image", (items.single() as Map<*, *>)["kind"])
        bridge.register("image")
        val ready = bridge.prepare(image, "image", "image", session) { _, _ -> }
        val decoded = BitmapFactory.decodeFile(ready.getValue("path"))
        assertNotNull(decoded)
        assertEquals(2, decoded.width)
        decoded.recycle()
    }

    @Test fun folderUrisPreserveEmptyDirectoriesAndBytes() {
        val tree = DocumentsContract.buildTreeDocumentUri(TestDocumentsProvider.AUTHORITY, TestDocumentsProvider.ROOT_ID)
        val root = DocumentsContract.buildDocumentUriUsingTree(tree, TestDocumentsProvider.ROOT_ID)
        val directory = requireNotNull(DocumentsContract.createDocument(resolver, root, DocumentsContract.Document.MIME_TYPE_DIR, "composer-folder"))
        additional.add(directory)
        DocumentsContract.createDocument(resolver, directory, DocumentsContract.Document.MIME_TYPE_DIR, "empty")
        val file = requireNotNull(DocumentsContract.createDocument(resolver, directory, "application/octet-stream", "child.bin"))
        resolver.openOutputStream(file, "w")!!.use { it.write(sourceBytes) }
        val folder = directory
        val items = bridge.read(ClipData.newRawUri("folder", folder))["items"] as List<*>
        assertEquals("folder", (items.single() as Map<*, *>)["kind"])
        bridge.register("folder")
        val ready = bridge.prepare(folder, "folder", "folder", session) { _, _ -> }
        val copied = File(ready.getValue("path"))
        assertEquals("composer-folder", copied.name)
        assertFalse(File(copied, "composer-source.bin").exists())
        assertTrue(File(copied, "empty").isDirectory)
        assertArrayEquals(sourceBytes, File(copied, "child.bin").readBytes())
    }

    @Test fun cancellationRemovesOnlyTheDraftCopy() {
        bridge.register("cancel")
        bridge.cancel("cancel")
        assertThrows(IllegalStateException::class.java) {
            bridge.prepare(source, "file", "cancel", session) { _, _ -> }
        }
        assertFalse(File(context.filesDir, "composer-sources/$session/cancel").exists())
        assertArrayEquals(sourceBytes, resolver.openInputStream(source)!!.use { it.readBytes() })
    }

    @Test fun revokedReadPermissionFailsWithoutPublishingAPartialSource() {
        TestDocumentsProvider.setFailure(context, "access", true)
        bridge.register("denied")
        assertThrows(Exception::class.java) {
            bridge.prepare(source, "file", "denied", session) { _, _ -> }
        }
        assertFalse(File(context.filesDir, "composer-sources/$session/denied").exists())
    }
}
