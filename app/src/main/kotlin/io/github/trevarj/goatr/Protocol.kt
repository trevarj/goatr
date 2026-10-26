// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.security.MessageDigest
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import kotlinx.serialization.*
import kotlinx.serialization.descriptors.*
import kotlinx.serialization.encoding.*
import kotlinx.serialization.json.*

typealias Text = String
interface WireValue { fun validate() }
fun invalidWire(): Nothing = throw ProtocolFailure(FailureKind.InvalidRequest)
fun requireWire(ok: Boolean) { if (!ok) invalidWire() }
fun utf8Size(s: String): Int { var n = 0; var i = 0; while (i < s.length) { val c = s[i++]; n += when { c.code < 128 -> 1; c.code < 2048 -> 2; c.isHighSurrogate() && i < s.length && s[i].isLowSurrogate() -> { i++; 4 }; else -> 3 } }; return n }
internal val uintToken = Regex("0|[1-9][0-9]*")
fun validUnicode(s: String): Boolean {
    var i = 0
    while (i < s.length) { val c = s[i++]; if (c.isHighSurrogate()) { if (i == s.length || !s[i++].isLowSurrogate()) return false } else if (c.isLowSurrogate()) return false }
    return true
}
fun Any?.validateWire() { when (this) { null -> {}; is WireValue -> validate(); is String -> requireWire(validUnicode(this) && utf8Size(this) <= Protocol.FRAME_MAX); is List<*> -> forEach { it.validateWire() }; is FieldValue -> when(this) { is FieldValue.Text -> value.validateWire(); is FieldValue.Multi -> values.validateWire(); is FieldValue.Flag -> {} } } }
enum class FailureKind { InvalidRequest, UnsupportedVersion, UnsupportedMethod, Incompatible }
class ProtocolFailure(val kind: FailureKind, val requestId: Uuid? = null, val close: Boolean = true) : Exception(kind.name)
data class Request(val id: Uuid, val body: RequestBody)
data class Response(val id: Uuid, val result: ResultBody? = null, val error: WireError? = null)
data class Event(val subscription: Uuid, val seq: Uint, val body: EventBody)
sealed interface Envelope { data class RequestFrame(val request: Request) : Envelope; data class ResponseFrame(val response: Response) : Envelope; data class EventFrame(val event: Event) : Envelope }

object FieldValueSerializer : KSerializer<FieldValue> {
    override val descriptor = JsonElement.serializer().descriptor
    override fun deserialize(decoder: Decoder): FieldValue {
        val v = (decoder as JsonDecoder).decodeJsonElement()
        return when { v is JsonArray -> FieldValue.Multi(v.map { requireWire(it is JsonPrimitive && it.isString); it.jsonPrimitive.content }); v is JsonPrimitive && v.isString -> FieldValue.Text(v.content); v is JsonPrimitive && v.booleanOrNull != null -> FieldValue.Flag(v.boolean); else -> invalidWire() }
    }
    override fun serialize(encoder: Encoder, value: FieldValue) { val v = when(value) { is FieldValue.Text -> JsonPrimitive(value.value); is FieldValue.Flag -> JsonPrimitive(value.value); is FieldValue.Multi -> JsonArray(value.values.map(::JsonPrimitive)) }; (encoder as JsonEncoder).encodeJsonElement(v) }
}

object Protocol {
    const val FRAME_MAX = 1048576
    const val JSON_DEPTH_MAX = 32
    const val JSON_MEMBERS_MAX = 16384
    const val UINT_MAX = 9007199254740991L
    const val ANSWER_MAX = 262144
    const val CONTENT_INLINE_MAX = 16384
    const val CONTENT_MAX = 16777216L
    internal val strictJson = Json { classDiscriminator = "kind"; encodeDefaults = true; explicitNulls = false }
    internal val responseJson = Json(strictJson) { ignoreUnknownKeys = true }
    fun parseRawJson(bytes: ByteArray): JsonElement {
        requireWire(bytes.isNotEmpty() && bytes.size <= FRAME_MAX)
        var validUtf8 = true
        // The Latin-1 error path can recover an ASCII UUID, but can never dispatch.
        val text = try { Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString() } catch (_: Exception) { validUtf8 = false; bytes.toString(Charsets.ISO_8859_1) }
        val scan = Scanner(text)
        try { scan.value(1); scan.whitespace(); requireWire(validUtf8 && scan.at == text.length && !scan.invalidStructure); val value = strictJson.parseToJsonElement(text); requireWire(validJsonUnicode(value)); return value } catch (e: Exception) { throw ProtocolFailure(FailureKind.InvalidRequest, scan.requestId) }
    }
    private class Scanner(val text: String) {
        var at = 0; var members = 0; var requestId: Uuid? = null; var invalidStructure = false; var ambiguousId = false
        fun whitespace() { while (at < text.length && text[at] in " \n\r\t") at++ }
        fun take(c: Char) { whitespace(); requireWire(at < text.length && text[at] == c); at++ }
        fun scanString() {
            take('"')
            while (true) { requireWire(at < text.length); val c = text[at++]; if (c == '"') break; if (c < ' ') invalidStructure = true; if (c == '\\') { requireWire(at < text.length); at++ } }
        }
        fun string(): String {
            whitespace(); val start = at; scanString()
            val result = strictJson.parseToJsonElement(text.substring(start, at)).jsonPrimitive.content
            requireWire(validUnicode(result)); return result
        }
        fun member() { members++; requireWire(members <= JSON_MEMBERS_MAX) }
        fun value(depth: Int) {
            whitespace(); requireWire(at < text.length)
            when (text[at]) {
                '{' -> { requireWire(depth <= JSON_DEPTH_MAX); at++; whitespace(); val keys = HashSet<String>(); if (at < text.length && text[at] == '}') { at++; return }
                    while (true) { member(); val key = string(); if (!keys.add(key)) { invalidStructure = true; if (depth == 1 && key == "id") { ambiguousId = true; requestId = null } }; take(':'); whitespace(); val start = at; value(depth + 1)
                        if (depth == 1 && key == "id" && !ambiguousId) requestId = try { strictJson.decodeFromString<Uuid>(text.substring(start, at)) } catch (_: Exception) { null }
                        whitespace(); if (at < text.length && text[at] == '}') { at++; break }; take(',') }
                }
                '[' -> { requireWire(depth <= JSON_DEPTH_MAX); at++; whitespace(); if (at < text.length && text[at] == ']') { at++; return }; while (true) { member(); value(depth + 1); whitespace(); if (at < text.length && text[at] == ']') { at++; break }; take(',') } }
                '"' -> scanString()
                't' -> { requireWire(text.startsWith("true", at)); at += 4 }
                'f' -> { requireWire(text.startsWith("false", at)); at += 5 }
                'n' -> { requireWire(text.startsWith("null", at)); at += 4 }
                in '0'..'9', '-' -> { val start = at; while (at < text.length && text[at] in "0123456789-+.eE") at++; val token = text.substring(start, at); if (!(uintToken.matches(token) && token.toLongOrNull()?.let { it <= UINT_MAX } == true)) invalidStructure = true }
                else -> invalidWire()
            }
        }
    }
    internal inline fun <reified T : WireValue> typed(v: JsonElement, strict: Boolean): T {
        try {
            val codec = if (strict) strictJson else responseJson
            knownNullFields(v, serializer<T>().descriptor)
            val result = codec.decodeFromJsonElement<T>(v); result.validate(); knownNulls(v, codec.encodeToJsonElement(result), strict); return result
        } catch (e: ProtocolFailure) { throw e } catch (_: Exception) { invalidWire() }
    }
    private fun validJsonUnicode(value: JsonElement): Boolean = when (value) {
        is JsonObject -> value.all { (key, v) -> validUnicode(key) && validJsonUnicode(v) }
        is JsonArray -> value.all(::validJsonUnicode)
        is JsonPrimitive -> !value.isString || validUnicode(value.content)
    }
    internal fun knownNulls(input: JsonElement, output: JsonElement, strict: Boolean) {
        when {
            input is JsonObject && output is JsonObject -> for ((k,v) in input) { if (k in output) { requireWire(v != JsonNull); knownNulls(v,output.getValue(k),strict) } else if (strict) invalidWire() }
            input is JsonArray && output is JsonArray -> { requireWire(input.size == output.size); for (i in input.indices) knownNulls(input[i],output[i],strict) }
            input is JsonPrimitive && output is JsonPrimitive -> { requireWire(input != JsonNull && output != JsonNull && input.isString == output.isString); if (!input.isString) requireWire((input.booleanOrNull != null) == (output.booleanOrNull != null)) }
            else -> invalidWire()
        }
    }
    @OptIn(ExperimentalSerializationApi::class)
    internal fun knownNullFields(input: JsonElement, descriptor: SerialDescriptor) {
        when (descriptor.kind) {
            StructureKind.CLASS, StructureKind.OBJECT -> if (input is JsonObject) for ((key, value) in input) {
                val i = descriptor.getElementIndex(key)
                if (i >= 0) { requireWire(value != JsonNull); knownNullFields(value, descriptor.getElementDescriptor(i)) }
            }
            StructureKind.LIST -> if (input is JsonArray) input.forEach { knownNullFields(it, descriptor.getElementDescriptor(0)) }
            PolymorphicKind.SEALED -> if (input is JsonObject) {
                val kind = (input["kind"] as? JsonPrimitive)?.content
                if (kind != null) { val variants = descriptor.getElementDescriptor(1); val i = variants.getElementIndex(kind); if (i >= 0) knownNullFields(input, variants.getElementDescriptor(i)) }
            }
            else -> {}
        }
    }
    /** Strict grid parts are decoded once, without re-serializing terminal cells for comparison. */
    internal fun gridPart(value:JsonElement):GridPart {
        fun shape(v:JsonElement) {
            when(v) {
                is JsonObject->for((key,child)in v){
                    if(key=="text" || key=="kind" || key=="underline")requireWire(child is JsonPrimitive && child.isString)
                    if(key=="bold" || key=="faint" || key=="italic" || key=="blink" || key=="strike")requireWire(child is JsonPrimitive && !child.isString && child.booleanOrNull!=null)
                    shape(child)
                }
                is JsonArray->v.forEach(::shape)
                else->requireWire(v!=JsonNull)
            }
        }
        shape(value)
        val part=strictJson.decodeFromJsonElement<GridPart>(value);part.validate();return part
    }
    fun decodePayload(bytes:ByteArray,responseMethod:Method?=null):Envelope =
        decodeParsed(parseRawJson(bytes) as? JsonObject ?: invalidWire(),responseMethod)
    internal fun decodeParsed(o:JsonObject,responseMethod:Method?=null):Envelope {
        val id = o["id"]?.let { try { strictJson.decodeFromJsonElement<Uuid>(it) } catch (_: Exception) { null } }
        val version = o["v"] as? JsonPrimitive
        if (version == null || version.isString || version.longOrNull == null) throw ProtocolFailure(FailureKind.InvalidRequest, id)
        if (version.long != 2L) throw ProtocolFailure(FailureKind.UnsupportedVersion, id)
        val kind = (o["type"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: throw ProtocolFailure(FailureKind.InvalidRequest, id)
        try {
            return when (kind) {
                "request" -> { requireWire(o.keys.all { it in setOf("v","type","id","method","params") }); val requestId = typed<Uuid>(o.getValue("id"), true); val params = o.getValue("params"); requireWire(params is JsonObject); val methodValue = o.getValue("method"); requireWire(methodValue is JsonPrimitive && methodValue.isString); val method = try { strictJson.decodeFromJsonElement<Method>(methodValue) } catch (_: Exception) { throw ProtocolFailure(FailureKind.UnsupportedMethod, requestId, false) }; Envelope.RequestFrame(Request(requestId, decodeRequestBody(method, params))) }
                "response" -> { requireWire(("result" in o) != ("error" in o)); Envelope.ResponseFrame(Response(typed(o.getValue("id"), false), o["result"]?.let { decodeResultBody(responseMethod ?: invalidWire(), it) }, o["error"]?.let { typed(it, false) })) }
                "event" -> { requireWire(o.keys.all { it in setOf("v","type","subscription","seq","kind","data") }); Envelope.EventFrame(Event(typed(o.getValue("subscription"), true), typed(o.getValue("seq"), true), decodeEventBody(o.getValue("kind").jsonPrimitive.content, o.getValue("data")))) }
                else -> invalidWire()
            }
        } catch (e: Exception) { val failure = e as? ProtocolFailure; throw ProtocolFailure(if (kind == "event" || kind == "response") FailureKind.Incompatible else failure?.kind ?: FailureKind.InvalidRequest, id, failure?.close ?: true) }
    }
    fun frameLength(header: ByteArray): Int { requireWire(header.size == 4); val n = ByteBuffer.wrap(header).int.toLong() and 0xffffffffL; requireWire(n in 1..FRAME_MAX.toLong()); return n.toInt() }
    private fun readExact(input: InputStream, n: Int): ByteArray { val bytes = ByteArray(n); var at = 0; while (at < n) { val count = input.read(bytes, at, n - at); requireWire(count > 0); at += count }; return bytes }
    fun readFrame(input: InputStream): ByteArray = readExact(input, frameLength(readExact(input, 4)))
    fun writeFrame(output: OutputStream, bytes: ByteArray) { requireWire(bytes.isNotEmpty() && bytes.size <= FRAME_MAX); output.write(ByteBuffer.allocate(4).putInt(bytes.size).array()); output.write(bytes) }
    private val scalarOrder = Comparator<String> { a,b -> val ai = a.codePoints().iterator(); val bi = b.codePoints().iterator(); var result = 0; while (ai.hasNext() && bi.hasNext()) { result = ai.nextInt().compareTo(bi.nextInt()); if (result != 0) break }; if (result != 0) result else ai.hasNext().compareTo(bi.hasNext()) }
    fun canonicalJson(value: JsonElement): ByteArray {
        val size = canonicalJsonSize(value); requireWire(size <= Int.MAX_VALUE)
        val out = StringBuilder(size.toInt())
        fun string(s: String) { requireWire(validUnicode(s)); out.append('"'); s.codePoints().forEach { c -> when(c) { 34 -> out.append("\\\""); 92 -> out.append("\\\\"); in 0..31 -> out.append("\\u").append(c.toString(16).padStart(4,'0')); else -> out.appendCodePoint(c) } }; out.append('"') }
        fun emit(v: JsonElement) { when(v) { is JsonObject -> { out.append('{'); v.keys.sortedWith(scalarOrder).forEachIndexed { i,k -> if (i > 0) out.append(','); string(k); out.append(':'); emit(v.getValue(k)) }; out.append('}') }; is JsonArray -> { out.append('['); v.forEachIndexed { i,x -> if (i>0) out.append(','); emit(x) }; out.append(']') }; is JsonPrimitive -> { if (v.isString) string(v.content) else { requireWire(v != JsonNull); if (v.booleanOrNull != null) out.append(v.content) else { requireWire(uintToken.matches(v.content)); val n = v.content.toLongOrNull() ?: invalidWire(); requireWire(n <= UINT_MAX); out.append(n) } } } } }
        emit(value); return out.toString().toByteArray(Charsets.UTF_8)
    }
    fun canonicalJsonSize(value: JsonElement): Long {
        fun string(s: String): Long { requireWire(validUnicode(s)); var n = 2L; var i = 0; while (i < s.length) { val c = s[i++]; n += when { c == '"' || c == '\\' -> 2; c.code < 32 -> 6; c.code < 128 -> 1; c.code < 2048 -> 2; c.isHighSurrogate() -> { i++; 4 }; else -> 3 } }; return n }
        return when(value) {
            is JsonObject -> 2L + maxOf(0,value.size-1) + value.entries.sumOf { (k,v) -> string(k) + 1 + canonicalJsonSize(v) }
            is JsonArray -> 2L + maxOf(0,value.size-1) + value.sumOf(::canonicalJsonSize)
            is JsonPrimitive -> if (value.isString) string(value.content) else if (value.booleanOrNull != null) if (value.boolean) 4 else 5 else { requireWire(value != JsonNull && uintToken.matches(value.content)); val n = value.content.toLongOrNull() ?: invalidWire(); requireWire(n <= UINT_MAX); value.content.length.toLong() }
        }
    }
    fun canonicalRequest(request: Request): ByteArray {
        request.id.validate(); val body = requestJson(request.body).toMutableMap()
        if (request.body.method == Method.SessionHistory || request.body.method == Method.NotificationsSync || request.body.method == Method.SessionQueueList) { val params = body.getValue("params").jsonObject.toMutableMap(); params.putIfAbsent("limit", JsonPrimitive(100)); body["params"] = JsonObject(params) }
        return canonicalJson(JsonObject(body))
    }
    fun sha256(bytes: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(bytes)
    fun hmacSha256(key: ByteArray, bytes: ByteArray): ByteArray = Mac.getInstance("HmacSHA256").run { init(SecretKeySpec(if (key.isEmpty()) ByteArray(64) else key, "HmacSHA256")); doFinal(bytes) }
    fun verifyHmacSha256(key: ByteArray, bytes: ByteArray, expected: ByteArray) = MessageDigest.isEqual(hmacSha256(key, bytes), expected)
    fun decodePresentation(bytes: ByteArray): Presentation = typed(parseRawJson(bytes),true)
    fun decodeAnswer(bytes: ByteArray): Answer = typed(parseRawJson(bytes),true)
    fun decodeFormField(bytes: ByteArray): FormField = typed(parseRawJson(bytes),true)
    fun encodePayload(envelope: Envelope): ByteArray {
        val objectValue = linkedMapOf<String,JsonElement>("v" to JsonPrimitive(2))
        when (envelope) {
            is Envelope.RequestFrame -> {
                val x = envelope.request; x.id.validate()
                objectValue["type"] = JsonPrimitive("request"); objectValue["id"] = strictJson.encodeToJsonElement(x.id)
                objectValue.putAll(requestJson(x.body))
            }
            is Envelope.ResponseFrame -> {
                val x = envelope.response; x.id.validate(); requireWire((x.result != null) != (x.error != null))
                objectValue["type"] = JsonPrimitive("response"); objectValue["id"] = strictJson.encodeToJsonElement(x.id)
                x.result?.let { objectValue["result"] = resultJson(it) }
                x.error?.let { it.validate(); objectValue["error"] = strictJson.encodeToJsonElement(it) }
            }
            is Envelope.EventFrame -> {
                val x = envelope.event; x.subscription.validate(); x.seq.validate()
                objectValue["type"] = JsonPrimitive("event"); objectValue["subscription"] = strictJson.encodeToJsonElement(x.subscription); objectValue["seq"] = strictJson.encodeToJsonElement(x.seq)
                objectValue.putAll(eventJson(x.body))
            }
        }
        var members = 0
        fun bounds(value: JsonElement, depth: Int) {
            val children = when (value) {
                is JsonObject -> value.values
                is JsonArray -> value
                else -> return
            }
            requireWire(depth <= JSON_DEPTH_MAX && children.size <= JSON_MEMBERS_MAX - members)
            members += children.size
            children.forEach { bounds(it, depth + 1) }
        }
        val value = JsonObject(objectValue)
        bounds(value, 1)
        val bytes = strictJson.encodeToString(value).toByteArray(Charsets.UTF_8)
        requireWire(bytes.size <= FRAME_MAX)
        return bytes
    }
}

@Serializable(with = UuidSerializer::class)
data class Uuid(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { java.util.UUID.fromString(s).let { it.toString() == s && it.variant() == 2 } } catch (_: Exception) { false }) }
    companion object { fun random(): Uuid = Uuid(java.util.UUID.randomUUID().toString()) }
}
object UuidSerializer : KSerializer<Uuid> {
    override val descriptor = PrimitiveSerialDescriptor("Uuid", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: Uuid) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = Uuid(decoder.decodeString())
}

@Serializable(with = EndpointIdSerializer::class)
data class EndpointId(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { computer.iroh.EndpointId.fromString(s).use { it.toString() == s } } catch (e: computer.iroh.IrohException) { e.close(); false }) }
}
object EndpointIdSerializer : KSerializer<EndpointId> {
    override val descriptor = PrimitiveSerialDescriptor("EndpointId", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: EndpointId) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = EndpointId(decoder.decodeString())
}

@Serializable(with = IdSerializer::class)
data class Id(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { s.isNotEmpty() && utf8Size(s) <= 512 } catch (_: Exception) { false }) }
}
object IdSerializer : KSerializer<Id> {
    override val descriptor = PrimitiveSerialDescriptor("Id", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: Id) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = Id(decoder.decodeString())
}

@Serializable(with = AgentNameSerializer::class)
data class AgentName(val value:String):WireValue {
    init { validate() }
    override fun validate() {requireWire(Regex("[a-z][a-z0-9_-]{0,31}").matches(value))}
}
object AgentNameSerializer:KSerializer<AgentName> {
    override val descriptor=PrimitiveSerialDescriptor("AgentName",PrimitiveKind.STRING)
    override fun serialize(encoder:Encoder,value:AgentName) {value.validate();encoder.encodeString(value.value)}
    override fun deserialize(decoder:Decoder)=AgentName(decoder.decodeString())
}

@Serializable(with = LabelSerializer::class)
data class Label(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { s.isNotEmpty() && utf8Size(s) <= 512 && s.codePointCount(0, s.length) <= 128 && s.codePoints().noneMatch { Character.isISOControl(it) } } catch (_: Exception) { false }) }
}
object LabelSerializer : KSerializer<Label> {
    override val descriptor = PrimitiveSerialDescriptor("Label", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: Label) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = Label(decoder.decodeString())
}

@Serializable(with = SafeMessageSerializer::class)
data class SafeMessage(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { utf8Size(s) <= 512 } catch (_: Exception) { false }) }
}
object SafeMessageSerializer : KSerializer<SafeMessage> {
    override val descriptor = PrimitiveSerialDescriptor("SafeMessage", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: SafeMessage) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = SafeMessage(decoder.decodeString())
}

@Serializable(with = InlineTextSerializer::class)
data class InlineText(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { utf8Size(s) <= Protocol.CONTENT_INLINE_MAX } catch (_: Exception) { false }) }
}
object InlineTextSerializer : KSerializer<InlineText> {
    override val descriptor = PrimitiveSerialDescriptor("InlineText", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: InlineText) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = InlineText(decoder.decodeString())
}

@Serializable(with = PromptTextSerializer::class)
data class PromptText(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { utf8Size(s) <= Protocol.ANSWER_MAX } catch (_: Exception) { false }) }
}
object PromptTextSerializer : KSerializer<PromptText> {
    override val descriptor = PrimitiveSerialDescriptor("PromptText", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: PromptText) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = PromptText(decoder.decodeString())
}

@Serializable(with = Key32Serializer::class)
data class Key32(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { s.length == 43 && java.util.Base64.getUrlDecoder().decode(s).let { it.size == 32 && java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(it) == s } } catch (_: Exception) { false }) }
}
object Key32Serializer : KSerializer<Key32> {
    override val descriptor = PrimitiveSerialDescriptor("Key32", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: Key32) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = Key32(decoder.decodeString())
}

@Serializable(with = Base64BytesSerializer::class)
data class Base64Bytes(val value: String) : WireValue {
    init { validate() }
    override fun validate() { val s = value; requireWire(validUnicode(s)); requireWire(try { java.util.Base64.getDecoder().decode(s).let { it.size <= 65536 && java.util.Base64.getEncoder().encodeToString(it) == s } } catch (_: Exception) { false }) }
}
object Base64BytesSerializer : KSerializer<Base64Bytes> {
    override val descriptor = PrimitiveSerialDescriptor("Base64Bytes", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: Base64Bytes) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = Base64Bytes(decoder.decodeString())
}

@Serializable(with = UintSerializer::class)
data class Uint(val value: Long) : WireValue { init { validate() }; override fun validate() { requireWire(value in 0..Protocol.UINT_MAX) } }
object UintSerializer : KSerializer<Uint> { override val descriptor = PrimitiveSerialDescriptor("Uint", PrimitiveKind.LONG); override fun serialize(encoder: Encoder, value: Uint) { value.validate(); encoder.encodeLong(value.value) }; override fun deserialize(decoder: Decoder) = Uint(decodeUint(decoder)) }

@Serializable(with = TrueValueSerializer::class)
data class TrueValue(val value: Boolean = true) : WireValue { init { validate() }; override fun validate() { requireWire(value == true) } }
object TrueValueSerializer : KSerializer<TrueValue> { override val descriptor = PrimitiveSerialDescriptor("TrueValue", PrimitiveKind.BOOLEAN); override fun serialize(encoder: Encoder, value: TrueValue) { value.validate(); encoder.encodeBoolean(value.value) }; override fun deserialize(decoder: Decoder): TrueValue { val p = (decoder as JsonDecoder).decodeJsonElement() as? JsonPrimitive ?: invalidWire(); requireWire(!p.isString && p.booleanOrNull != null); return TrueValue(p.boolean) } }

@Serializable(with = OneValueSerializer::class)
data class OneValue(val value: Long = 1L) : WireValue { init { validate() }; override fun validate() { requireWire(value == 1L) } }
object OneValueSerializer : KSerializer<OneValue> { override val descriptor = PrimitiveSerialDescriptor("OneValue", PrimitiveKind.LONG); override fun serialize(encoder: Encoder, value: OneValue) { value.validate(); encoder.encodeLong(value.value) }; override fun deserialize(decoder: Decoder) = OneValue(decodeUint(decoder)) }

@Serializable(with = ZeroValueSerializer::class)
data class ZeroValue(val value: Long = 0L) : WireValue { init { validate() }; override fun validate() { requireWire(value == 0L) } }
object ZeroValueSerializer : KSerializer<ZeroValue> { override val descriptor = PrimitiveSerialDescriptor("ZeroValue", PrimitiveKind.LONG); override fun serialize(encoder: Encoder, value: ZeroValue) { value.validate(); encoder.encodeLong(value.value) }; override fun deserialize(decoder: Decoder) = ZeroValue(decodeUint(decoder)) }

@Serializable(with = UnknownValueSerializer::class)
data class UnknownValue(val value: String = "unknown") : WireValue { init { validate() }; override fun validate() { requireWire(value == "unknown") } }
object UnknownValueSerializer : KSerializer<UnknownValue> { override val descriptor = PrimitiveSerialDescriptor("UnknownValue", PrimitiveKind.STRING); override fun serialize(encoder: Encoder, value: UnknownValue) { value.validate(); encoder.encodeString(value.value) }; override fun deserialize(decoder: Decoder) = UnknownValue(decoder.decodeString()) }

private fun decodeUint(decoder: Decoder): Long {
    val p = (decoder as JsonDecoder).decodeJsonElement() as? JsonPrimitive ?: invalidWire()
    requireWire(!p.isString && uintToken.matches(p.content))
    val n = p.content.toLongOrNull() ?: invalidWire()
    requireWire(n <= Protocol.UINT_MAX)
    return n
}

@Serializable(with = ProtocolVersionSerializer::class)
data class ProtocolVersion(val value: Long = 2L) : WireValue { init { validate() }; override fun validate() { requireWire(value == 2L) } }
object ProtocolVersionSerializer : KSerializer<ProtocolVersion> { override val descriptor = PrimitiveSerialDescriptor("ProtocolVersion", PrimitiveKind.LONG); override fun serialize(encoder: Encoder, value: ProtocolVersion) { value.validate(); encoder.encodeLong(value.value) }; override fun deserialize(decoder: Decoder) = ProtocolVersion(decodeUint(decoder)) }
