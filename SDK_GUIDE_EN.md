# Kernel Script SDK Integration Guide

This guide is for developers integrating Kernel Script into a Rust application. It covers dependencies, driver lifecycle, API behavior, error handling, threading, and safety boundaries.

For the Lua API, see [document.md](document.md) or [document_CN.md](document_CN.md). This guide covers the Rust SDK.

> Translation scope: Operational instructions for manual driver mapping, executable-memory patching, and shellcode detours are omitted. The relevant sections describe behavior and limitations.

## 1. Architecture and mental model

```text
Your application
  └─ ks-sdk (driver lifecycle and SDK facade)
       └─ ks-link (process enumeration and ring-buffer requests)
            └─ ks-core (protocol and ring layout, no_std)
                 └─ ks-driver (a kernel worker thread handles requests)
```

- Driver requests use a synchronous ring-buffer round trip. The source guide reports about 60–100 µs per call. Keep these requests off render threads and other real-time paths.
- The application communicates with the driver through shared memory. The section and event DACLs grant access to SYSTEM and Administrators, so the client must run elevated.
- The driver permits only one live instance. A remaining instance can prevent a new one from starting.

## 2. Requirements

- Windows 10 or 11, x64. The source guide reports validation on build 26300.
- Rust 1.85 or later and the MSVC toolchain. The WDK is needed to build the driver itself.
- The SDK's lifecycle implementation has a separate driver-loader dependency. Windows security policy can prevent driver operations from succeeding.

## 3. Add the SDK dependency

For a Rust project with a sibling Kernel Script checkout, add the SDK to your `Cargo.toml`:

```toml
[dependencies]
ks-sdk = { path = "../kernel-script/ks-sdk" }
```

`ks-sdk` re-exports the `ks-link` API and protocol constants from `ks-core`. Most callers can use a single `ks_sdk::` import path.

The SDK embeds a driver image from `ks-sdk/assets/ks-driver.sys` at build time. The source guide says the artifact is excluded from Git by the `*.sys` ignore rule.

## 4. Driver lifecycle

On startup, the SDK checks for state left by a previous run and can replace a responsive instance. Startup waits for the driver to publish its communication objects. A successful result indicates that the ring is available. A stale instance that cannot be stopped may still hold the single-instance marker and cause startup to fail with `0xC0000035`.

Shutdown stops the worker and waits for the single-instance marker to be released. Coordinate startup and shutdown across clients that share the same driver.

The original guide lists environment variables for configuring the loader. Those settings are outside the scope of this translation.

## 5. Error handling

`ks_sdk::Error` covers driver lifecycle operations:

| Variant | Meaning |
|---|---|
| `DriverEntry(u32)` | NTSTATUS returned by the driver's entry point. `0xC0000035` can indicate that an existing instance still holds the single-instance marker. |
| `NotReady { .. }` | The driver loaded, but the readiness signal did not arrive within five seconds. |
| `ScLoad(u32)` | The driver could not be loaded. Signing or system policy may be involved. |
| `Setup(String)` | A setup step failed, such as preparing a temporary directory or changing the working directory. |
| `Link(LinkError)` | A ring-buffer request during shutdown failed. |

`ks_sdk::LinkError` covers ring-buffer operations:

| Variant | Meaning |
|---|---|
| `DriverNotReady { object, code }` | A shared section or event is missing, or access was denied. Win32 error code 5 means access denied. |
| `RingIncompatible` | The ring exists, but its magic value or version differs from the client's. Check that the client and driver image match. |
| `NtStatus(i32)` | The driver returned an NTSTATUS error. |
| `TimedOut` | The driver did not respond within the five-second cancellation window. |
| `TooManyEntries { limit }` | The batch exceeded the per-request entry limit. |
| `Encode` / `Decode` | Request encoding or response decoding failed. |
| `WinApi { operation, code }` | An underlying Win32 call failed. |

`ProcessError` covers process enumeration through a Toolhelp snapshot. This operation runs in user mode.

## 6. Processes and modules

Use user-mode enumeration to list processes and resolve a process name to a PID.

```rust
for info in ks_sdk::list()? {
    println!("{} ({})", info.name, info.pid);
}
```

`get_process_base` reports a process's module base from the driver's perspective. Windows reuses PIDs. A process can exit between lookup and use, and a later request can fail with `NtStatus`.

## 7. Memory reads and writes

The API distinguishes absolute addresses from relative virtual addresses (RVAs). An RVA is interpreted relative to the process's module base.

The SDK rejects transfers larger than `MAX_DRIVER_TRANSFER_SIZE` (4,096 bytes) on the client side. Writes to shared physical pages can affect other processes that map the same image.

## 8. Batches and pointer chains

A batch read returns tightly packed bytes in input-address order, with no separators. A batch write returns one NTSTATUS value per entry in input order. Zero indicates success.

The documented limits are 256 entries for a batch read, 64 entries for a batch write, and 32 levels for a pointer chain. Each pointer-chain step dereferences a 64-bit pointer.

## 9. Memory allocation and region queries

The documented maximum allocation size is 16 MiB. Windows rounds allocations up to whole pages. Allocations belong to the target process and are released when it exits.

**Freeing a region is destructive.** The driver does not track whether a region was originally allocated through the SDK. Passing an unrelated allocation base can remove a mapping that the application still needs.

### Page protection and region queries

A region query reports the region base, allocation base, size, state, protection, allocation protection, and type. The wire representation is a fixed 40-byte little-endian record.

Protection changes operate on whole pages and require the entire requested span to be mapped. The reported previous protection describes the first page. Restoring a range whose original protection varied across regions requires tracking each region's protection individually. Windows can reject changes under its dynamic code or integrity policies.

## 10. Memory locks

A memory lock keeps a region at a configured value by repeating writes in the driver worker between requests. The driver maintains up to 64 locks. A lock ID is unique within the driver and must be nonzero.

The lock list is driver-wide. It includes entries from different clients, such as the GUI and Lua scripts, and reports each entry's ID, PID, address, and length. Shutdown discards the table and stops the repeated writes.

## 11. Suspend and resume

Suspending a process stops all of its threads. The SDK rejects the idle process, the system process, and the calling process. Suspending the caller would deadlock the request.

Repeated suspension of a process already suspended by this driver is idempotent. The driver only resumes a process that it suspended itself. **Driver shutdown forgets the suspend records and leaves affected processes suspended.** Balance each suspension with a resume before shutdown.

## 12. Kernel hooks

The source guide describes hook registration, enabling and disabling, detours, restoration, and driver self-tests. Hooking changes executable code. An unsupported instruction sequence can cause an operation to fail. Memory integrity protections can prevent changes, and a faulty patch can crash the system.

The driver documents restoring patched code before releasing its hook resources. Its self-tests are scoped to functions owned by the driver itself.

## 13. Concurrency and threading

- The shared ring has one request slot and a client mutex. Calls from multiple threads are serialized, with one request in flight at a time.
- Each process establishes its session lazily on the first request. Reloading the driver changes the shared object names, leaving the previous session disconnected.
- Call `ks_sdk::close_session()` only when no requests are in flight. Gate new requests before a lifecycle job closes or replaces a session. The shared `ks-ui-common` layer uses this pattern for `ks-gui` and `ks-inspector`.
- A stale session fails its next request and requires explicit reinitialization.

## 14. Safety boundaries and limitations

- The shared section and event DACLs grant access to SYSTEM and Administrators. A client without the required access receives `ERROR_ACCESS_DENIED`.
- There can be only one live driver instance. The source guide gives conflicting descriptions of repeated startup. Section 4 describes replacing a responsive instance, while the original safety section describes a collision. Check the behavior of the build you are using.
- Shutdown is destructive to the current session. It stops the worker and discards driver-side lock and hook state. Further requests fail until a driver and session are available.
- Unloading a driver does not resume processes it previously suspended.
- Windows can reuse PIDs. Resolve a PID close to the operation that uses it.
- Driver calls block their calling thread. Keep them off latency-sensitive paths.

## 15. Tests

The repository includes `ks-test.exe` for end-to-end SDK verification and `ks-hook-test.exe` for hook-specific verification. The full SDK suite includes concurrent requests, shutdown, and session recreation. The hook suite includes driver-owned self-tests and a detour test in the harness's own process.

## 16. License and provenance

The driver lifecycle component depends on a private Rust port of a KDU loader. The original guide states that the private mapper source, third-party driver binaries, and associated shellcode must stay out of the public repository.
