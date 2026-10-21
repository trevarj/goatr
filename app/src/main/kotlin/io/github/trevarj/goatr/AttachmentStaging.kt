// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.system.Os
import android.system.OsConstants
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.nio.file.Files
import java.nio.file.LinkOption
import java.security.KeyStore
import java.security.MessageDigest
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Local picker correlation only. The connection owner must still admit the original opaque scope. */
internal class AttachmentSourceTicket(val id:UUID=UUID.randomUUID(), internal val originalScope:Any) {
    override fun toString()="AttachmentSourceTicket(redacted)"
}
internal enum class AttachmentPreparationFailure { Unavailable, Empty, TooLarge, Capacity, Changed, Expired }
internal class AttachmentPreparationException(val reason:AttachmentPreparationFailure):Exception("Attachment preparation unavailable")
internal class PreparedAttachment internal constructor(
    val id:UUID,val displayName:String,val mediaType:String,val size:Long,val sha256:String,val expiresAt:Long,
    internal val device:Long,internal val inode:Long,internal val ciphertextSize:Long,
) { override fun toString()="PreparedAttachment(redacted)" }

/** Streaming encryption under noBackupFilesDir. Source URIs and display metadata remain in memory.
 * No remote action, URI permission persistence, provider path or insertion authority lives here. */
internal class AttachmentStaging(context:Context) {
    companion object {
        const val MAX_BYTES=8L*1024*1024
        const val MAX_CHUNK=128*1024
        private const val MAX_FILES=32
        private const val MAX_LOCAL_BYTES=64L*1024*1024
        private const val LIFETIME=60L*60*1000
        private const val ALIAS="goatr.attachments.staging.v1"
        private const val HEADER_BYTES=21
        private val mutex=Mutex()
        private val NAME=Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\\.gcm")
    }
    private val app=context.applicationContext
    private val directory=File(app.noBackupFilesDir.canonicalFile,"attachment-staging")
    private val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
    private fun fail(reason:AttachmentPreparationFailure=AttachmentPreparationFailure.Unavailable):Nothing=throw AttachmentPreparationException(reason)
    private fun root() {
        val fresh=!Files.exists(directory.toPath(),LinkOption.NOFOLLOW_LINKS)
        if(fresh && !directory.mkdir())fail()
        if(fresh)Os.chmod(directory.path,0x1c0)
        val stat=Os.lstat(directory.path)
        if(directory.canonicalFile!=directory.absoluteFile || !OsConstants.S_ISDIR(stat.st_mode) || stat.st_uid!=Os.getuid() || stat.st_mode and 0xfff!=0x1c0)fail()
    }
    private fun key():SecretKey {
        if(keys.containsAlias(ALIAS))return keys.getKey(ALIAS,null) as? SecretKey ?: fail()
        // Existing encrypted material never authorizes silently replacing its lost key.
        if(directory.listFiles()?.isNotEmpty()!=false)fail()
        val generator=KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore")
        generator.init(KeyGenParameterSpec.Builder(ALIAS,KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true).build())
        return generator.generateKey()
    }
    private fun file(id:UUID)=File(directory,"$id.gcm")
    private fun check(path:File):android.system.StructStat {
        val stat=Os.lstat(path.path)
        if(!OsConstants.S_ISREG(stat.st_mode) || stat.st_uid!=Os.getuid() || stat.st_mode and 0xfff!=0x180 || stat.st_nlink!=1L || stat.st_size !in 0..MAX_BYTES+HEADER_BYTES+16)fail()
        return stat
    }
    private fun checkCapacity() {
        val files=directory.listFiles() ?: fail()
        if(files.size>=MAX_FILES)fail(AttachmentPreparationFailure.Capacity)
        var total=0L
        for(file in files){if(!NAME.matches(file.name))fail();total+=check(file).st_size}
        // Reserve a complete maximum-size ciphertext before opening a source stream.
        if(total+MAX_BYTES+HEADER_BYTES+16>MAX_LOCAL_BYTES)fail(AttachmentPreparationFailure.Capacity)
    }
    private fun displayName(value:String?):String {
        val out=StringBuilder();var bytes=0
        for(code in (value ?: "Attachment").take(256).codePoints().toArray()){
            if(Character.isISOControl(code) || code in 0xD800..0xDFFF || code==47 || code==92 || Character.getType(code)==Character.FORMAT.toInt())continue
            val text=String(Character.toChars(code));val size=text.toByteArray(Charsets.UTF_8).size
            if(bytes+size>128)break
            out.append(text);bytes+=size
        }
        return out.toString().trim().takeIf {it.isNotEmpty() && it!="." && it!=".."} ?: "Attachment"
    }
    private fun mediaType(value:String?):String=value?.lowercase(java.util.Locale.ROOT)?.takeIf {
        it.toByteArray(Charsets.UTF_8).size<=128 && it.matches(Regex("[a-z0-9!#$&^_.+-]+/[a-z0-9!#$&^_.+-]+"))
    } ?: "application/octet-stream"
    private fun digest(bytes:ByteArray)=bytes.joinToString(""){"%02x".format(it.toInt() and 255)}
    private fun aad(id:UUID,header:ByteArray)=id.toString().toByteArray(Charsets.UTF_8)+header
    private fun same(path:File,prepared:PreparedAttachment):Boolean=try {val stat=check(path);stat.st_dev==prepared.device && stat.st_ino==prepared.inode && stat.st_size==prepared.ciphertextSize}catch(_:Exception){false}

    private suspend fun <T> guardedPreparation(block:suspend ()->T):T=try {block()}catch(cancelled:kotlinx.coroutines.CancellationException){throw cancelled}catch(error:AttachmentPreparationException){throw error}catch(_:Exception){fail()}

    suspend fun prepare(uri:Uri,progress:(Long)->Unit={}):PreparedAttachment {
        var owned:PreparedAttachment?=null
        var delivered=false
        try {val result=guardedPreparation {withContext(Dispatchers.IO) {
        mutex.withLock {
            root();checkCapacity();val secret=key();val id=UUID.randomUUID();val path=file(id)
            val expires=System.currentTimeMillis()+LIFETIME
            val cipher=Cipher.getInstance("AES/GCM/NoPadding").apply {init(Cipher.ENCRYPT_MODE,secret)}
            if(cipher.iv.size!=12)fail()
            val header=ByteBuffer.allocate(HEADER_BYTES).put(1.toByte()).putLong(expires).put(cipher.iv).array()
            cipher.updateAAD(aad(id,header))
            // Reported SIZE is never used as the byte bound. No source URI is retained after this call.
            val label=try {app.contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME),null,null,null)?.use {cursor->if(cursor.moveToFirst())displayName(cursor.getString(0))else "Attachment"} ?: "Attachment"}catch(_:Exception){"Attachment"}
            val mime=try {mediaType(app.contentResolver.getType(uri))}catch(_:Exception){"application/octet-stream"}
            val fd=Os.open(path.path,OsConstants.O_WRONLY or OsConstants.O_CREAT or OsConstants.O_EXCL or OsConstants.O_NOFOLLOW,0x180)
            val original=Os.fstat(fd);var complete=false
            try {
                val hash=MessageDigest.getInstance("SHA-256");var size=0L
                FileOutputStream(fd).use {output->
                    output.write(header)
                    val buffer=ByteArray(32*1024)
                    try {
                        (app.contentResolver.openInputStream(uri) ?: fail()).use {input->
                            while(true){currentCoroutineContext().ensureActive();val count=input.read(buffer);if(count<0)break;if(count==0)continue
                                if(size+count>MAX_BYTES)fail(AttachmentPreparationFailure.TooLarge)
                                hash.update(buffer,0,count);cipher.update(buffer,0,count)?.let(output::write);size+=count;progress(size)
                            }
                        }
                        if(size==0L)fail(AttachmentPreparationFailure.Empty)
                        output.write(cipher.doFinal());output.flush();output.fd.sync()
                    }finally{buffer.fill(0)}
                }
                val stat=check(path)
                if(stat.st_dev!=original.st_dev || stat.st_ino!=original.st_ino)fail(AttachmentPreparationFailure.Changed)
                currentCoroutineContext().ensureActive()
                complete=true
                PreparedAttachment(id,label,mime,size,digest(hash.digest()),expires,stat.st_dev,stat.st_ino,stat.st_size).also {owned=it}
            }finally{
                if(!complete){try {val stat=Os.lstat(path.path);if(stat.st_dev==original.st_dev && stat.st_ino==original.st_ino)Os.remove(path.path)}catch(_:Exception){}}
            }
        }
    }}
            delivered=true;return result
        }finally{if(!delivered)owned?.let {file->kotlinx.coroutines.withContext(kotlinx.coroutines.NonCancellable){discard(file)}}}
    }

    /** Returns at most one canonical chunk only after the entire ciphertext tag/size/digest verifies.
     * The bounded scan prevents unauthenticated GCM prefix bytes from reaching a network caller. */
    suspend fun readChunk(prepared:PreparedAttachment,offset:Long,limit:Int=MAX_CHUNK):ByteArray=guardedPreparation {withContext(Dispatchers.IO) {
        mutex.withLock {
            require(offset>=0 && offset<prepared.size && limit in 1..MAX_CHUNK)
            root();if(System.currentTimeMillis()>=prepared.expiresAt)fail(AttachmentPreparationFailure.Expired)
            val path=file(prepared.id);if(!same(path,prepared))fail(AttachmentPreparationFailure.Changed)
            val fd=Os.open(path.path,OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW,0)
            val stat=Os.fstat(fd);if(stat.st_dev!=prepared.device || stat.st_ino!=prepared.inode){Os.close(fd);fail(AttachmentPreparationFailure.Changed)}
            val selected=ByteArray(minOf(limit.toLong(),prepared.size-offset).toInt());var returned=false
            try {
                FileInputStream(fd).use {input->
                    val header=ByteArray(HEADER_BYTES);var at=0
                    while(at<header.size){val n=input.read(header,at,header.size-at);if(n<=0)fail();at+=n}
                    val fields=ByteBuffer.wrap(header);if(fields.get().toInt()!=1 || fields.long!=prepared.expiresAt)fail()
                    val iv=ByteArray(12).also {fields.get(it)}
                    val cipher=Cipher.getInstance("AES/GCM/NoPadding").apply {init(Cipher.DECRYPT_MODE,key(),GCMParameterSpec(128,iv));updateAAD(aad(prepared.id,header))}
                    var total=0L;var encryptedTotal=HEADER_BYTES.toLong();val hash=MessageDigest.getInstance("SHA-256");val encrypted=ByteArray(32*1024)
                    fun consume(plain:ByteArray){try {if(total+plain.size>prepared.size)fail();hash.update(plain)
                        val start=maxOf(offset,total);val end=minOf(offset+selected.size,total+plain.size)
                        if(start<end)plain.copyInto(selected,(start-offset).toInt(),(start-total).toInt(),(end-total).toInt())
                        total+=plain.size
                    }finally{plain.fill(0)}}
                    try {while(true){currentCoroutineContext().ensureActive();val count=input.read(encrypted);if(count<0)break;if(count==0)continue;encryptedTotal+=count;if(encryptedTotal>prepared.ciphertextSize)fail(AttachmentPreparationFailure.Changed);cipher.update(encrypted,0,count)?.let(::consume)};consume(cipher.doFinal())}finally{encrypted.fill(0)}
                    if(encryptedTotal!=prepared.ciphertextSize || total!=prepared.size || digest(hash.digest())!=prepared.sha256 || !same(path,prepared))fail(AttachmentPreparationFailure.Changed)
                }
                returned=true;selected
            }finally{if(!returned)selected.fill(0)}
        }
    }
    }
    /** Orphan expiry requires authenticating the whole bounded object and its UUID/header AAD. */
    suspend fun sweepExpired(retained:Set<String>) {try {withContext(Dispatchers.IO){mutex.withLock {
        root();val secret=key();val files=directory.listFiles() ?: return@withLock
        if(files.size>MAX_FILES)return@withLock
        for(path in files){currentCoroutineContext().ensureActive();if(!NAME.matches(path.name))continue
            val id=try {UUID.fromString(path.name.removeSuffix(".gcm"))}catch(_:Exception){continue};if(id.toString() in retained)continue
            try {val original=check(path);val fd=Os.open(path.path,OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW,0)
                FileInputStream(fd).use {input->if(Os.fstat(fd).st_ino!=original.st_ino||Os.fstat(fd).st_dev!=original.st_dev)fail()
                    val header=ByteArray(HEADER_BYTES);var at=0;while(at<header.size){val count=input.read(header,at,header.size-at);if(count<=0)fail();at+=count}
                    val fields=ByteBuffer.wrap(header);if(fields.get().toInt()!=1)fail();val expires=fields.long
                    if(expires>System.currentTimeMillis()||expires<=0)return@use
                    val iv=ByteArray(12).also {fields.get(it)};val cipher=Cipher.getInstance("AES/GCM/NoPadding").apply {init(Cipher.DECRYPT_MODE,secret,GCMParameterSpec(128,iv));updateAAD(aad(id,header))}
                    var bytes=HEADER_BYTES.toLong();var plainBytes=0L;val buffer=ByteArray(32768)
                    try {while(true){currentCoroutineContext().ensureActive();val count=input.read(buffer);if(count<0)break;if(count==0)continue;bytes+=count;if(bytes>original.st_size)fail();cipher.update(buffer,0,count)?.let {plain->plainBytes+=plain.size;plain.fill(0);if(plainBytes>MAX_BYTES)fail()}};cipher.doFinal().let {plain->plainBytes+=plain.size;plain.fill(0)}
                        if(bytes!=original.st_size||plainBytes !in 1..MAX_BYTES)fail()
                        val current=check(path);if(current.st_dev==original.st_dev&&current.st_ino==original.st_ino&&current.st_size==original.st_size)Os.remove(path.path)
                    }finally{buffer.fill(0)}
                }
            }catch(cancel:kotlinx.coroutines.CancellationException){throw cancel}catch(_:Exception){}
        }
    }}}catch(cancel:kotlinx.coroutines.CancellationException){throw cancel}catch(_:Exception){}}
    suspend fun discard(prepared:PreparedAttachment):Boolean=try {withContext(Dispatchers.IO) {mutex.withLock {
        root();val path=file(prepared.id)
        if(!Files.exists(path.toPath(),LinkOption.NOFOLLOW_LINKS))return@withLock true
        if(!same(path,prepared))return@withLock false
        Os.remove(path.path);true
    }}}catch(cancelled:kotlinx.coroutines.CancellationException){throw cancelled}catch(_:Exception){false}
}
