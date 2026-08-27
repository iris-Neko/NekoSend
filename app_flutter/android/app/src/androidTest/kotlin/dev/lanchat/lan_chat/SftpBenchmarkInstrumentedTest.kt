package dev.lanchat.lan_chat

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.TimeUnit
import org.apache.sshd.client.SshClient
import org.apache.sshd.client.keyverifier.AcceptAllServerKeyVerifier
import org.apache.sshd.common.util.io.PathUtils
import org.apache.sshd.sftp.client.SftpClient
import org.apache.sshd.sftp.client.SftpClientFactory
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SftpBenchmarkInstrumentedTest {
    @Test
    fun uploadsConfiguredFileAndReportsThroughput() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val arguments = InstrumentationRegistry.getArguments()
        assumeTrue(
            "SFTP benchmark requires the host instrumentation argument",
            !arguments.getString("host").isNullOrBlank(),
        )
        val host = requireArgument(arguments.getString("host"), "host")
        val port = arguments.getString("port")?.toIntOrNull() ?: 22022
        val username = requireArgument(arguments.getString("username"), "username")
        val password = requireArgument(arguments.getString("password"), "password")
        val sourceName = arguments.getString("sourceName") ?: "reverse-perf-10g.bin"
        val remotePath = arguments.getString("remotePath") ?: "/android-sftp-upload.bin"
        val source = instrumentation.targetContext.filesDir.resolve(sourceName)
        check(source.exists()) { "benchmark source is missing: ${source.absolutePath}" }

        PathUtils.setUserHomeFolderResolver {
            instrumentation.targetContext.filesDir.toPath()
        }
        val client = SshClient.setUpDefaultClient().apply {
            serverKeyVerifier = AcceptAllServerKeyVerifier.INSTANCE
            start()
        }
        try {
            client.connect(username, host, port)
                .verify(15, TimeUnit.SECONDS)
                .session.use { session ->
                    session.addPasswordIdentity(password)
                    session.auth().verify(15, TimeUnit.SECONDS)
                    SftpClientFactory.instance().createSftpClient(session).use { sftp ->
                        val started = System.nanoTime()
                        val result = if (source.isDirectory) {
                            uploadDirectory(sftp, source, remotePath)
                        } else {
                            uploadFile(sftp, source, remotePath)
                            UploadResult(source.length(), 1)
                        }
                        val elapsedSeconds = (System.nanoTime() - started) / 1_000_000_000.0
                        val megabytesPerSecond = result.bytes / 1_000_000.0 / elapsedSeconds
                        val report = "SFTP_BENCHMARK bytes=${result.bytes} " +
                            "files=${result.files} " +
                            "elapsed_seconds=${"%.3f".format(elapsedSeconds)} " +
                            "megabytes_per_second=${"%.3f".format(megabytesPerSecond)}"
                        Log.i(TAG, report)
                        println(report)
                    }
                }
        } finally {
            client.stop()
        }
    }

    private fun requireArgument(value: String?, name: String): String =
        requireNotNull(value?.takeIf { it.isNotBlank() }) {
            "instrumentation argument '$name' is required"
        }

    private fun uploadDirectory(
        sftp: SftpClient,
        source: File,
        remotePath: String,
    ): UploadResult {
        val remoteRoot = remotePath.trimEnd('/')
        sftp.mkdir(remoteRoot)
        var totalBytes = 0L
        var fileCount = 0
        source.walkTopDown().drop(1).forEach { entry ->
            val relative = entry.relativeTo(source).invariantSeparatorsPath
            val target = "$remoteRoot/$relative"
            if (entry.isDirectory) {
                sftp.mkdir(target)
            } else {
                uploadFile(sftp, entry, target)
                totalBytes = Math.addExact(totalBytes, entry.length())
                fileCount += 1
            }
        }
        return UploadResult(totalBytes, fileCount)
    }

    private fun uploadFile(sftp: SftpClient, source: File, remotePath: String) {
        source.inputStream().buffered(BUFFER_SIZE).use { input ->
            sftp.write(
                remotePath,
                SftpClient.OpenMode.Write,
                SftpClient.OpenMode.Create,
                SftpClient.OpenMode.Truncate,
            ).buffered(BUFFER_SIZE).use { output ->
                input.copyTo(output, BUFFER_SIZE)
            }
        }
        assertEquals(source.length(), sftp.stat(remotePath).size)
    }

    private data class UploadResult(val bytes: Long, val files: Int)

    companion object {
        private const val TAG = "LanChatSftpBenchmark"
        private const val BUFFER_SIZE = 1024 * 1024
    }
}
