// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import kotlinx.serialization.*
import kotlinx.serialization.descriptors.*
import kotlinx.serialization.encoding.*

@Serializable(with = DecimalSerializer::class)
@ConsistentCopyVisibility
data class Decimal private constructor(val value: String) : Comparable<Decimal>, WireValue {
    companion object {
        fun parse(text: String): Decimal {
            requireWire(text.toByteArray(Charsets.UTF_8).size <= 128 && Regex("-?(0|[1-9][0-9]*)(\\.[0-9]+)?").matches(text))
            val unsigned = text.removePrefix("-")
            val whole = unsigned.substringBefore('.')
            val fraction = unsigned.substringAfter('.', "").trimEnd('0')
            val zero = whole == "0" && fraction.isEmpty()
            return Decimal((if (text.startsWith('-') && !zero) "-" else "") + whole + (if (fraction.isEmpty()) "" else ".$fraction"))
        }
        fun fromNativeToken(token: String): Decimal {
            requireWire(token.length <= Protocol.FRAME_MAX && Regex("-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?").matches(token))
            val at = token.indexOfAny(charArrayOf('e', 'E'))
            val mantissa = if (at < 0) token else token.substring(0, at)
            val unsigned = mantissa.removePrefix("-")
            val whole = unsigned.substringBefore('.')
            val digits = whole + unsigned.substringAfter('.', "")
            val first = digits.indexOfFirst { it != '0' }
            if (first < 0) return parse("0")
            val exponent = if (at < 0) 0 else token.substring(at + 1).toIntOrNull() ?: invalidWire()
            val significant = digits.substring(first, digits.indexOfLast { it != '0' } + 1)
            val point = whole.length.toLong() + exponent - first
            val size = if (point <= 0) 2 - point + significant.length else if (point >= significant.length) point else significant.length + 1L
            requireWire(size + (if (token.startsWith('-')) 1 else 0) <= 128)
            val body = when {
                point <= 0 -> "0." + "0".repeat((-point).toInt()) + significant
                point >= significant.length -> significant + "0".repeat((point - significant.length).toInt())
                else -> significant.substring(0, point.toInt()) + "." + significant.substring(point.toInt())
            }
            return parse((if (token.startsWith('-')) "-" else "") + body)
        }
    }
    val isInteger: Boolean get() = !value.contains('.')
    override fun validate() { requireWire(parse(value).value == value) }
    override fun compareTo(other: Decimal): Int {
        val negative = value.startsWith('-')
        if (negative != other.value.startsWith('-')) return if (negative) -1 else 1
        val a = value.removePrefix("-"); val b = other.value.removePrefix("-")
        val ai = a.substringBefore('.'); val bi = b.substringBefore('.')
        var order = ai.length.compareTo(bi.length)
        if (order == 0) order = ai.compareTo(bi)
        val af = a.substringAfter('.', ""); val bf = b.substringAfter('.', "")
        if (order == 0) for (i in 0 until maxOf(af.length, bf.length)) {
            order = af.getOrElse(i) { '0' }.compareTo(bf.getOrElse(i) { '0' })
            if (order != 0) break
        }
        return if (negative) -order else order
    }
}
object DecimalSerializer : KSerializer<Decimal> {
    override val descriptor = PrimitiveSerialDescriptor("Decimal", PrimitiveKind.STRING)
    override fun serialize(encoder: Encoder, value: Decimal) { value.validate(); encoder.encodeString(value.value) }
    override fun deserialize(decoder: Decoder) = Decimal.parse(decoder.decodeString())
}

@OptIn(ExperimentalSerializationApi::class)
fun Decimal.nativeJsonNumber(): kotlinx.serialization.json.JsonPrimitive {
    validate()
    return kotlinx.serialization.json.JsonUnquotedLiteral(value)
}
