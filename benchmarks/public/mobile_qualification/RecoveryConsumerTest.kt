import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.writerslogic.warm.DocumentInput
import com.writerslogic.warm.SearchOptions
import com.writerslogic.warm.StoreOptions
import com.writerslogic.warm.WarmStore
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.CountDownLatch
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RecoveryConsumerTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val mode = InstrumentationRegistry.getArguments().getString("case").also {
        require(it == "plain" || it == "encrypted")
    }!!
    private val root = File(context.filesDir, "recovery-$mode")
    private val options = StoreOptions(
        dimensions = 1024u,
        encryptionKey = if (mode == "encrypted") "qualification-fixture-passphrase" else null
    )
    private val storePath get() = File(root, "store").absolutePath

    private fun synced(file: File, value: String, append: Boolean = false) {
        FileOutputStream(file, append).use { stream ->
            stream.write(value.toByteArray(Charsets.UTF_8))
            stream.fd.sync()
        }
    }

    private fun input(id: Int, generation: Int) = DocumentInput(
        id = "d$id",
        text = "recovery record$id generation$generation",
        metadataJson = """{"ordinal":$id,"generation":$generation}"""
    )

    private fun checkpoint(store: WarmStore, phase: String, acknowledgement: String) {
        store.snapshotBlocking(File(root, "snapshot-$phase").absolutePath)
        synced(File(root, "acks.ndjson"), acknowledgement + "\n", true)
    }

    @Test
    fun writer() {
        check(root.mkdir())
        val store = WarmStore.openBlocking(storePath, options)
        assertEquals(List(32) { 1u }, store.ingestBatchBlocking((0 until 32).map { input(it, 0) }))
        checkpoint(store, "inserted", """{"phase":"inserted","documents":32}""")
        assertEquals(List(16) { 1u }, store.ingestBatchBlocking((0 until 16).map { input(it, 1) }))
        checkpoint(store, "replaced", """{"phase":"replaced","documents":16}""")
        for (id in 24 until 32) assertTrue(store.deleteDocumentBlocking("d$id"))
        checkpoint(store, "ready", """{"phase":"ready","deleted":8}""")
        synced(File(root, "ready"), "ready\n")
        CountDownLatch(1).await()
        error("writer must be terminated by the host")
    }

    @Test
    fun reader() {
        val store = WarmStore.openBlocking(storePath, options)
        assertEquals(24uL, store.stats().documentCount)
        for (id in 0 until 32) {
            val hits = store.searchBlocking("recovery", SearchOptions(documentIds = listOf("d$id")))
            if (id >= 24) {
                assertFalse(store.hasDocument("d$id"))
                assertTrue(hits.isEmpty())
            } else {
                val generation = if (id < 16) 1 else 0
                assertTrue(store.hasDocument("d$id"))
                assertEquals(1, hits.size)
                assertEquals("d$id", hits.single().documentId)
                assertEquals("recovery record$id generation$generation", hits.single().text)
                val metadata = JSONObject(hits.single().metadataJson)
                assertEquals(id, metadata.getInt("ordinal"))
                assertEquals(generation, metadata.getInt("generation"))
            }
        }
        assertEquals(1u, store.ingestBlocking(DocumentInput(id = "resumed", text = "recovery resumed")))
        store.snapshotBlocking(File(root, "snapshot-resumed").absolutePath)
        store.shutdown()
        val reopened = WarmStore.openBlocking(storePath, options)
        assertEquals(25uL, reopened.stats().documentCount)
        assertEquals(listOf("resumed"), reopened.searchBlocking("resumed").map { it.documentId })
        for (id in 24 until 32) assertFalse(reopened.hasDocument("d$id"))
        reopened.shutdown()
        synced(File(root, "verified.json"), """{"phase":"verified","records":24,"deleted":8,"resumedWrite":true}""")
    }
}
