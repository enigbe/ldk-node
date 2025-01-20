/*
 * This Kotlin source file was generated by the Gradle 'init' task.
 */
package org.lightningdevkit.ldknode

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlin.io.path.createTempDirectory
import kotlin.test.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.*

@RunWith(AndroidJUnit4::class)
class AndroidLibTest {
    @Test
    fun node_start_stop() {
        val tmpDir1 = createTempDirectory("ldk_node").toString()
        println("Random dir 1: $tmpDir1")
        val tmpDir2 = createTempDirectory("ldk_node").toString()
        println("Random dir 2: $tmpDir2")

        val listenAddress1 = "127.0.0.1:2323"
        val listenAddress2 = "127.0.0.1:2324"

        val config1 = defaultConfig()
        config1.storageDirPath = tmpDir1
        config1.listeningAddresses = listOf(listenAddress1)
        config1.network = Network.REGTEST

        val config2 = defaultConfig()
        config2.storageDirPath = tmpDir2
        config2.listeningAddresses = listOf(listenAddress2)
        config2.network = Network.REGTEST

        val builder1 = Builder.fromConfig(config1)
        val builder2 = Builder.fromConfig(config2)

        val node1 = builder1.build()
        val node2 = builder2.build()

        node1.start()
        node2.start()

        val nodeId1 = node1.nodeId()
        println("Node Id 1: $nodeId1")

        val nodeId2 = node2.nodeId()
        println("Node Id 2: $nodeId2")

        val address1 = node1.onchain_payment().newOnchainAddress()
        println("Funding address 1: $address1")

        val address2 = node2.onchain_payment().newOnchainAddress()
        println("Funding address 2: $address2")

        node1.stop()
        node2.stop()
    }
}
