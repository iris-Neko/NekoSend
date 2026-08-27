package dev.lanchat.lan_chat

import android.net.Uri
import android.provider.DocumentsContract
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SafReceiveInstrumentedTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val targetContext = instrumentation.targetContext
    private val providerContext = targetContext
    private val treeUri: Uri = DocumentsContract.buildTreeDocumentUri(
        TestDocumentsProvider.AUTHORITY,
        TestDocumentsProvider.ROOT_ID,
    )

    @Before
    fun setUp() {
        TestDocumentsProvider.setFailure(providerContext, "delete", false)
        TestDocumentsProvider.setFailure(providerContext, "rename", false)
        TestDocumentsProvider.setFailure(providerContext, "access", false)
        TestDocumentsProvider.setFailure(providerContext, "space", false)
        providerContext.cacheDir.resolve("instrumented-documents").deleteRecursively()
        targetContext.getSharedPreferences(TEST_PREFERENCES, 0).edit().clear().commit()
    }

    @After
    fun tearDown() {
        TestDocumentsProvider.setFailure(providerContext, "delete", false)
        TestDocumentsProvider.setFailure(providerContext, "rename", false)
        TestDocumentsProvider.setFailure(providerContext, "access", false)
        TestDocumentsProvider.setFailure(providerContext, "space", false)
        targetContext.getSharedPreferences(TEST_PREFERENCES, 0).edit().clear().commit()
    }

    @Test
    fun folderPreparationReusesRootEmptyDirectoryAndPartialAfterRecreate() {
        val entries = listOf(
            entry("entry-root", "directory", "photos", 0),
            entry("entry-empty", "directory", "photos/empty", 0),
            entry("entry-file", "file", "photos/data.bin", 8),
        )
        val firstPrepared = preparedById(
            adapter().prepareReceiveTree(treeUri, entries),
        )
        val partial = Uri.parse(firstPrepared.getValue("entry-file")["partialRef"] as String)
        targetContext.contentResolver.openFileDescriptor(partial, "rw")!!.use { descriptor ->
            java.io.FileOutputStream(descriptor.fileDescriptor).use { it.write(byteArrayOf(1, 2, 3)) }
        }

        val secondPrepared = preparedById(
            adapter().prepareReceiveTree(treeUri, entries),
        )
        assertEquals(
            firstPrepared.getValue("entry-root")["destinationRef"],
            secondPrepared.getValue("entry-root")["destinationRef"],
        )
        assertEquals(
            firstPrepared.getValue("entry-empty")["destinationRef"],
            secondPrepared.getValue("entry-empty")["destinationRef"],
        )
        assertEquals(
            firstPrepared.getValue("entry-file")["partialRef"],
            secondPrepared.getValue("entry-file")["partialRef"],
        )
        assertEquals(3L, (secondPrepared.getValue("entry-file")["persistedOffset"] as Number).toLong())
    }

    @Test
    fun completedFileCommitIsIdempotentWhenProviderChangesDocumentId() {
        val entries = listOf(entry("entry-single", "file", "answer.bin", 4))
        val adapter = adapter()
        val prepared = preparedById(adapter.prepareReceiveTree(treeUri, entries)).getValue("entry-single")
        val partial = Uri.parse(prepared["partialRef"] as String)
        targetContext.contentResolver.openFileDescriptor(partial, "rw")!!.use { descriptor ->
            java.io.FileOutputStream(descriptor.fileDescriptor).use { it.write(byteArrayOf(4, 3, 2, 1)) }
        }
        val first = adapter.commitReceiveFile(partial, prepared["destinationRef"] as String)
        val second = adapter().commitReceiveFile(
            partial,
            prepared["destinationRef"] as String,
        )
        assertEquals(first, second)
        assertEquals("answer.bin", queryName(first))
    }

    @Test
    fun failedProbeDoesNotLeaveAFalseSuccessfulCapabilityResult() {
        TestDocumentsProvider.setFailure(providerContext, "delete", true)
        val failure = try {
            adapter().probeReceiveTree(treeUri)
            null
        } catch (error: Throwable) {
            error
        }
        assertNotNull(failure)
        assertFalse(failure is AssertionError)
    }

    @Test
    fun revokedProviderAccessMapsToPermissionLost() {
        TestDocumentsProvider.setFailure(providerContext, "access", true)
        val failure = try {
            adapter().probeReceiveTree(treeUri)
            null
        } catch (error: Throwable) {
            error
        }
        assertNotNull(failure)
        assertEquals("FILE_PERMISSION_LOST", androidPlatformErrorCode(requireNotNull(failure)))
    }

    @Test
    fun providerStorageLimitReturnsEnospcThroughTheSafDescriptor() {
        val prepared = preparedById(
            adapter().prepareReceiveTree(
                treeUri,
                listOf(entry("entry-space", "file", "space.bin", 2L * 1024L * 1024L)),
            ),
        ).getValue("entry-space")
        TestDocumentsProvider.setFailure(providerContext, "space", true)

        val failure = try {
            val partial = Uri.parse(prepared.getValue("partialRef") as String)
            targetContext.contentResolver.openFileDescriptor(partial, "rw")!!.use { descriptor ->
                java.io.FileOutputStream(descriptor.fileDescriptor).use { output ->
                    output.write(ByteArray(1024 * 1024))
                    output.write(1)
                    output.fd.sync()
                }
            }
            null
        } catch (error: Throwable) {
            error
        }

        assertNotNull(failure)
        assertTrue(requireNotNull(failure).toString().contains("ENOSPC", ignoreCase = true))
    }

    private fun preparedById(result: Map<String, Any>): Map<String, Map<String, Any?>> {
        @Suppress("UNCHECKED_CAST")
        val prepared = result.getValue("prepared") as List<Map<String, Any?>>
        return prepared.associateBy { it.getValue("entryId") as String }
    }

    private fun adapter(): SafReceiveAdapter = SafReceiveAdapter(
        targetContext,
        TEST_PREFERENCES,
    )

    private fun queryName(uri: Uri): String = targetContext.contentResolver.query(
        uri,
        arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME),
        null,
        null,
        null,
    )!!.use { cursor ->
        check(cursor.moveToFirst())
        cursor.getString(0)
    }

    private fun entry(id: String, kind: String, path: String, size: Long): Map<String, Any?> =
        mapOf(
            "entryId" to id,
            "entryKind" to kind,
            "relativePath" to path,
            "size" to size,
            "persistedOffset" to 0L,
        )

    companion object {
        private const val TEST_PREFERENCES = "lan_chat_platform_test"
    }
}
