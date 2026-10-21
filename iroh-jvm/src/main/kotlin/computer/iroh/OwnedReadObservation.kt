// SPDX-License-Identifier: GPL-3.0-or-later
package computer.iroh

import java.util.concurrent.atomic.AtomicReference

/** Closed diagnostic state for one caller-owned readExact future. */
public enum class OwnedReadPhase {
    NotStarted,
    Entered,
    PollRequested,
    Wake,
    Ready,
    CompletionEntered,
    CompletionReturned,
}

/**
 * Retains only the monotonic boundary state and the callback lifetime for one existing future.
 * It does not retain a native handle, callback data, bytes, or error value.
 */
public class OwnedReadObservation {
    private val current = AtomicReference(OwnedReadPhase.NotStarted)
    private val callback = AtomicReference<UniffiRustFutureContinuationCallback?>(null)

    public fun phase(): OwnedReadPhase = current.get()

    internal fun advance(next: OwnedReadPhase) {
        while (true) {
            val previous = current.get()
            if (previous.ordinal >= next.ordinal || current.compareAndSet(previous, next)) return
        }
    }

    internal fun retain(value: UniffiRustFutureContinuationCallback) {
        callback.set(value)
    }

    internal fun release(value: UniffiRustFutureContinuationCallback?) {
        if (value != null) callback.compareAndSet(value, null)
    }
}

/** Delegates exactly one generated RecvStream.readExact future while retaining closed poll state. */
public suspend fun RecvStream.readExactObserved(
    size: UInt,
    observation: OwnedReadObservation,
): ByteArray {
    observation.advance(OwnedReadPhase.Entered)
    var observedCallback: UniffiRustFutureContinuationCallback? = null
    return uniffiRustCallAsync(
        callWithHandle { handle ->
            UniffiLib.uniffi_iroh_ffi_fn_method_recvstream_read_exact(
                handle,
                FfiConverterUInt.lower(size),
            )
        },
        { future, callback, continuation ->
            observation.advance(OwnedReadPhase.PollRequested)
            val bridge = object : UniffiRustFutureContinuationCallback {
                override fun callback(data: Long, pollResult: Byte) {
                    when (pollResult) {
                        UNIFFI_RUST_FUTURE_POLL_READY -> observation.advance(OwnedReadPhase.Ready)
                        UNIFFI_RUST_FUTURE_POLL_WAKE -> observation.advance(OwnedReadPhase.Wake)
                    }
                    callback.callback(data, pollResult)
                }
            }
            observedCallback = bridge
            observation.retain(bridge)
            UniffiLib.ffi_iroh_ffi_rust_future_poll_rust_buffer(future, bridge, continuation)
        },
        { future, continuation ->
            observation.advance(OwnedReadPhase.CompletionEntered)
            val completed = UniffiLib.ffi_iroh_ffi_rust_future_complete_rust_buffer(future, continuation)
            observation.advance(OwnedReadPhase.CompletionReturned)
            completed
        },
        { future ->
            try {
                UniffiLib.ffi_iroh_ffi_rust_future_free_rust_buffer(future)
            } finally {
                observation.release(observedCallback)
            }
        },
        { FfiConverterByteArray.lift(it) },
        IrohException.ErrorHandler,
    )
}
