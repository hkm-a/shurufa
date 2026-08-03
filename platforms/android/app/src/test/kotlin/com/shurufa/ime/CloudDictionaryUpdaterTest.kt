package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.security.MessageDigest

class CloudDictionaryUpdaterTest {
    @Test
    fun advertised_download_size_respects_limit() {
        assertEquals(true, CloudDictionaryUpdater.fitsDownloadLimit(-1, 32))
        assertEquals(true, CloudDictionaryUpdater.fitsDownloadLimit(32, 32))
        assertEquals(false, CloudDictionaryUpdater.fitsDownloadLimit(33, 32))
    }

    @Test
    fun manifest_requires_https_and_safe_yaml_path() {
        assertThrows(IllegalStateException::class.java) {
            CloudDictionaryUpdater.parseManifest(
                """{"version":1,"revision":"r1","files":[{"path":"../bad.yaml","url":"https://example.com/bad","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":1}]}""".toByteArray(),
            )
        }
        assertThrows(IllegalStateException::class.java) {
            CloudDictionaryUpdater.parseManifest(
                """{"version":1,"revision":"r1","files":[{"path":"dict.yaml","url":"http://example.com/dict","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":1}]}""".toByteArray(),
            )
        }
    }

    @Test
    fun manifest_and_content_hash_roundtrip() {
        val content = "自定义词库".toByteArray()
        val hash = MessageDigest.getInstance("SHA-256").digest(content).joinToString("") { "%02x".format(it) }
        val manifest = CloudDictionaryUpdater.parseManifest(
            """{"version":1,"revision":"r2","files":[{"path":"cn_dicts/custom.yaml","url":"https://example.com/dict","fallback_urls":["https://mirror.example.com/dict"],"sha256":"$hash","size":${content.size}}]}""".toByteArray(),
        )
        assertEquals("r2", manifest.revision)
        assertEquals(listOf("https://mirror.example.com/dict"), manifest.files.single().fallbackUrls)
        CloudDictionaryUpdater.verify(manifest.files.single(), content)
    }
}
