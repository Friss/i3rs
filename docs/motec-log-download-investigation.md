# MoTeC log download investigation

Experimental native M1 discovery, inspection, raw acquisition and conversion to i3rs `.ld` recordings are implemented. A physical M150 download and a comparison with a saved M1 Tune image were completed on 2026-11-24. Compatibility across M1 hardware and firmware remains unverified.

## Current feature

The native desktop app provides **File → Download M1 logs…**. Scan for devices or enter an IP endpoint, inspect logging systems and permissions, choose systems and an existing destination folder, then download and convert. Saved `.m1dli` images and complete i3rs raw bundles can also be converted locally. The window reports progress, supports cancellation and retains the output folder on failure. Scanning only browses advertisements; inspection and acquisition require explicit actions. Logging systems are displayed as 1–8; protocol and manifest IDs are 0–7.

Each download creates a unique child directory containing `manifest.json` and exact received Mo records (`.mo`). The format identifier is `i3rs.m1.raw-capture.v1`. This is an i3rs investigation bundle, **not** an official `.m1dli`, `.m1dli-archive`, or `.ld` file. The app does not send these bundles to the recording loader. The raw manifest explicitly records `produces_ld=false` and `converter_validated=false`. Conversion creates a separate child directory containing fresh `.ld` files and an `i3rs.m1.converted-recordings.v1` manifest. Original raw records remain intact. The conversion manifest records its resampling policy, warnings, file completion and `vendor_compatibility_validated=false`.

The acquisition transport exposes CONNECT, SET_MTA, UPLOAD, MoTeC Fast Upload and DISCONNECT. It rechecks active-user download permissions on the acquisition connection before reading sectors. Missing or malformed permissions fail closed. Credentials, unlock/key negotiation and protected-device support remain unfinished; no authentication bypass, erase, or programming command is implemented.

CLI examples (substitute the advertised endpoint, including IPv6 scope and port):

```bash
cargo run --release -p i3rs-cli -- m1 devices --window-ms 3000
cargo run --release -p i3rs-cli -- m1 inspect '[fe80::1%4]:5555'
cargo run --release -p i3rs-cli -- m1 download-raw '[fe80::1%4]:5555' --systems 1,2 --destination /existing/captures
cargo run --release -p i3rs-cli -- m1 image-info /path/to/image.m1dli
cargo run --release -p i3rs-cli -- m1 convert-image /path/to/image.m1dli --destination /existing/recordings
cargo run --release -p i3rs-cli -- m1 convert-capture /path/to/raw-bundle --destination /existing/recordings
cargo run --release -p i3rs-cli -- m1 download '[fe80::1%4]:5555' --systems 1,2 --destination /existing/captures
```

The sample endpoint is illustrative; no fixed device port is assumed. `image-info` checks an outer raw-image record and bounded plain metadata only. It does not decode channels or convert files. Native acquisition is excluded from the web build.

## Official behavior

M1 Tune retrieval uses Ctrl+F8 or Online → Retrieve Logged Data. Users select logging systems and an image destination, choose whether to retain or erase device logging, enter details, and optionally convert the acquired image. The conversion screenshot allows disconnecting the device and deferring conversion through File → Convert a Logged Data Image. Acquisition must therefore not be modeled as receiving an already usable `.ld` file. [Retrieve Logged Data](https://help.motec.com.au/m1/tune/1.5/Topics/Retrieve_Logged_Data_1.5.html), [conversion screenshot](https://help.motec.com.au/m1/tune/1.5/Images/Convert%20logged%20data%20dialog.PNG).

M1 communication uses Ethernet/IPv6; Discovery supplies the available ECU list. [Connection](https://help.motec.com.au/m1/tune/1.5/Topics/Connecting_the_M1_ECU_to_the_PC_1.5.html), [establishing communications](https://help.motec.com.au/m1/tune/1.5/Topics/Establishing_ECU_Communications_1.5.html). Eight logging systems have distinct access controls, including the ability to retrieve an image without permission to convert it. [Logging features](https://help.motec.com.au/m1/tune/1.5/Topics/Logging_Features_1.5.html).

i2's File → Get Logged Data and optional automatic loading after unload are documented in MoTeC training. This does not establish a common acquisition protocol across hardware families. [Training, p.169](https://www.motec.com.au/hessian/uploads/2015_DTC_Colour_Displays_v2_0_d7cc55fc75.pdf). C127 Dash Manager and M84 ECU Manager have their own workflows; M84 uses UTC/CAN. [C127 manual](https://www.motec.com.au/hessian/uploads/C127_User_Manual_da85a94ff5.pdf), [M84 manual](https://www.motec.com.au/hessian/uploads/M84_Manual_A5_bd59be464c.pdf).

Tune 1.5's File → Export Data is a separate time-graph telemetry export with activation requirements and up to 50 Hz output. It is not evidence for onboard-log retrieval. [Data Export](https://help.motec.com.au/m1/tune/1.5/Topics/Data_Export_1.5.html).

## Static evidence and identity

Research material is under ignored `target/motec-download-re/`; it is machine-local and is not a committed evidence archive. SHA-256 identities:

| Artifact | SHA-256 |
| --- | --- |
| Official installer | `dad33e013b88a87bc6211654eb524cfd234f587370bd5574eea1600ecdf2510f` |
| `payload/m1tune.exe` | `8b30d6e497813d81521646c04b78e658bcfc32708ef90231984131609df6d76d` |
| `payload/m1tune.chm` | `7163a46a23e3661a5bec647480adfbec28ef0c7a2bfa0753a109ffae9f011fe9` |
| `payload/motec.discovery.exe.*` | `163dd537823a5089ab40e189fccab7865c27ca8cd1d88af42a2924125ef1fca8` |
| `payload/motec.discovery.dll` | `2c8053bbbf32ace31d654a2c14c3324b571cd882f36130f471addf3e18341af5` |
| `payload/motec.extract.exe.*` | `1fd911e0a66bfd7f35b62b0337356d667223c449ecedc0aa5d87d2d2cca46bfe` |

### REA analysis method

Initial VMware guest file access was unavailable, so the official M1 Tune 1.4.0.0302 installer was downloaded and extracted offline. Later, PowerShell and a temporary receiver limited to the guest IP copied the two selected raw images and one Tune reference into the local ignored research directory. The receiver was stopped after transfer. No vendor software was installed on the host. Analysis used REA with Ghidra 12.1.4 against immutable PE bytes. Initial default analysis exceeded the startup bound. A JVM thread dump identified expensive analysis passes; a temporary per-process profile disabled `WindowsResourceReference` and `X86 Function Callee Purge`. No REA client configuration was changed. Decompiled code has the usual type recovery and calling-convention limitations; findings below were cross-checked at callers and related functions where necessary.

Retained complete REA records are in `target/motec-download-re/m1tune-focused-evidence.json`, the focused query outbox, and `discovery-exe-evidence.json` / `discovery-dll-evidence.json`. These machine-local ignored files are not committed fixtures. Addresses below use Tune's PE image base `0x400000`.

## Evidence ledger

| Evidence / function | Observation and implementation boundary |
| --- | --- |
| `ev_2f14714e26122ba9ca7dc03b50ca033e78f16412c69736507686f0a4e8bb3e4f`, `0x479840` | Tune's discovery setup registers `_motec-xcp-e7._udp` and `_motec-15._udp`. Both are browsed. DNS-SD supplies endpoint ports; scoped IPv6 interfaces and raw TXT bytes are preserved. Advertisements are candidates, not proof of compatible hardware. |
| `ev_edd5dc2d59f8cdbddc840a89b9654171a94e000e05ae181ab4d413caa5dcc28c`, Discovery `0x434350` | Generic `_motec-%02X._udp` formatting with `0x2f`. **The meaning of type `0x2f` remains unknown; it is not identified as M1.** |
| `ev_e9711d5cbe574392a3e5d03daa80374bc9e7973ddcb291cbe6c45937010df19b`, `0xe554d0` | Four-byte envelope: little-endian u16 payload length and u16 request counter, beginning at 1. `0xe551b0` receives envelopes. `0xe555d0` is a destructor, not a batch receiver. |
| `ev_458b2957323f143c8300995bf5a39b46346a0454141e2f13cc9456c99ba82d99`, `0xe57d60` / `0xe58040` | Replies correlate by echoed request counter; FF/FE command responses are dispatched separately from events/DAQ. Stale replies and non-command traffic do not restart the deadline. |
| `ev_55539ba12b39c26aca203321044e745319bc5c4f988553f1380a19eac90d0602`, `0xe4c3e0` | CONNECT reader consumes resources, communication flags, CTO and DTO fields through byte 5. Trailing bytes remain opaque. Byte order and address granularity are negotiated. |
| `0x7796f0`, `0x775980`, `0x77a490`, `0x775f60`, `0x777c50`, `0x779f30` | FF success; FE with zero error is accepted; other FE errors rejected. SET_MTA carries extension at byte 3 and negotiated-endian address at 4–7. UPLOAD counts address units and skips address-granularity bytes in replies. Data bytes remain unmodified. |
| `ev_4c6983852354eaaf1efbcd9d3dc75ff940102dec5a957b14ebc7c645a2d651b0`, `ev_5ee54c875a338d13692a46b6756c39700521b9b7383ee47e78ab5e4cc0ac83c7`, `0x771fa0` / `0x772600` | Extension `0x26`, selector `0x00010062` reads log information: 12-byte Mo header followed by its declared payload using the same MTA. Setups use `0x27/0`, metadata `0x2a/0`. |
| `ev_a3557f30d1e3018229a069b71fdc82e8f6df666353fab92b10452717a15ead42`, `0x6a3580` | Log-info root `0x0200002e`; system leaves `0x0201002f`, indexed by header byte 2. Big-endian boundaries A/B at payload offsets 8/12. |
| `ev_84ceffc74b7cd99a8bbeb9897794fd0fff599f411191b38cdf815dfc14e40eb1`, `0x6a33d0` | Device leaf `0x02010033`; optional mapping numerator at 16, last sector at 40, denominator at 44. Modern mapping uses rounded Windows MulDiv semantics. Legacy extension `0x25` packs system and sector into the address. |
| `ev_6eccc772eacc450bf8579a8825846bc4faf99a5d06f6484a2c94a283c54d5c8d`, `0x687ab0` | Active-user `0x0200004b` contains permissions `0x02010010`; download bit is payload `[64 + system] & 1`. Fallback permissions selector is `0x00010040` on extension `0x26`. |
| `0x88bd00`, `0x88f040` | Pipeline reads information, checks security, reads setups, metadata and sectors. Sector traversal is newest first within each selected range. Optional erase `0x88d970` is a separate stage and is deliberately absent from i3rs. |
| `ev_8b4139c0a7ece2e64906b21c8e2f6b69bb71023f0d1b6f0de5ba5e6d2a47498d`, `ev_5f3bf25d9f9f6d14225f95ab17c0d5c593233d700f32ce7ca8a70d289cc5b3e6`, `0x6a9c50` / `0xabfc60` | Mo header: `Mo` signature, control bytes 2/3, big-endian type at 4 and payload length at 8. Raw-image root type `0x01000005`. Bounds and exact outer span are checked before parsing. |
| `0x6a9960`, `0xabea40` | Image children include source package `0x0100000e`, property groups `0x01000009`, setups `0x02000020`, metadata `0x02000046`, enables `0x02010012`, stream `0x01010002`. Property name/value are child positions 0/1, not header-byte-2 identifiers. |
| `0xac5030` | Byte 3 selects plain or transformed payload handling. Encoding 31 is now decoded as a big-endian uncompressed-length prefix followed by one checked XZ stream; the expanded bytes must form the matching plain Mo record. Other nonzero encodings remain unsupported. Signatures and encrypted images remain unvalidated. |
| `ev_58fdd514317da6fe3e52a672c33638c42b37eed93427f1e8922ccda10abe5260`, `0x90cf00` | Conversion resolves a matching Data Logging Configuration. i3rs joins setup metadata GUIDs to the corresponding catalog before packet decoding. Image signatures and conversion permissions remain uninterpreted and are reported as warnings. |

## Native conversion findings

Discovery, transport, retrieval, image decoding, native conversion and GUI integration have synthetic and local-file coverage. See the M150 validation below for physical acquisition results.

Two real M1 images were copied from the VMware guest. The larger image contains numeric telemetry; M1 Tune successfully converted it to a reference LD in the guest. No fixture bytes or vendor executable templates are committed. Ignored research results are under `target/motec-download-re/fixtures/20240125/`.

Observed image and packet structure:

- Child Mo records align to four bytes; alignment padding is outside each record's declared payload. Setup GUIDs select packet layouts, while separate metadata GUIDs select TuneConfig XML catalogs.
- Metadata encoding 31 expands to a plain Mo record containing TuneConfig XML. The source package also contains A2L calibration data; the inspected A2L has no `MEASUREMENT` blocks and is unnecessary for the established telemetry mapping.
- Setup entries specify `(base, address, count)` and expand consecutive four-byte channel words. The numeric fixture has 156 entries yielding 196 channels in seven groups at 1, 2, 5, 10, 20, 50 and 100 Hz.
- Numeric packet `0x0201002b` contains big-endian low ticks, big-endian 32-bit sample words, then CRC32/ISO-HDLC. Every inspected numeric packet passed this checksum. Timing packets supply the high ticks; the default 125 MHz clock follows the vendor fallback and remains explicit in manifests.
- XML scalar types `f32`, `u32`, `s32`, `enum` and `s32e7` determine decoding. GPS uses signed integers times `1e-7`; enums retain signed values. TuneConfig address metadata, names, enum labels and explicit `DefaultLocale` units supply the catalog. Known affine unit pairs include deg/s→rpm, ratio→%, ratio→%Trim and the observed metric/time conversions. Unknown requested pairs retain their physical units and produce a warning only when the channel is selected.

The vendor reference has 196 channels and a 180.61-second presentation timeline. Raw channel names matched after resolving chained `ItemValue` aliases. Engine Speed, Throttle Position and Coolant Temperature samples matched sequentially after locale conversion, with the expected float32 rounding. Dynamic 10 Hz channels had one additional vendor sample repeating their final value. This establishes these mappings for one inspected image; it does not establish universal converter parity.

Timing is an intentional native departure. Tune floors the first packet's clock origin to a whole ECU second (`0x90fbd0`) and shortens gaps over 30 seconds to a rounded previous time plus five seconds (`0x9109c0`, globals `0x13d0498=30,000,000 µs`, `0x13d04a0=5,000,000 µs`). The real image's 64.3496-second gap between consecutive 20 Hz samples becomes 6.35 seconds in Tune, removing 58 seconds. i3rs retains original elapsed time and starts each reset/power segment at its first numeric sample. It writes separate recordings for those segments and reports the timing difference.

Fresh LD writing preserves a common segment origin. Floating values are linearly interpolated only between adjacent samples no more than 1.5 periods apart; integer/state channels hold the previous value for at most 1.5 periods. Missing coverage is NaN. This explicit policy is recorded in every conversion manifest; it is not a claim to reproduce Tune's grid assignment. Output uses float64 engineering values, retains large unsigned values exactly, and keeps full names in attributes. Signed enum labels outside the legacy range use an explicitly versioned private `i3rs` attribute. These extended LD files reopen in i3rs; their interoperability with i2/Tune remains unvalidated.

Unknown or incomplete areas include protected image handling, conversion permission semantics, board-specific clocks, calendar recording timestamps, metadata switching between different catalogs, unsupported sample encodings and parity across firmware versions. Unsupported structure fails explicitly instead of producing guessed telemetry. Image enable/permission records and signatures remain uninterpreted and are disclosed in conversion warnings.

## Implementation details

- `i3rs-core::m1::xcp` owns strict framing, negotiated protocol fields and read-only commands. The codec can structurally decode consecutive envelopes; that is not a claim that a particular ECU batches them.
- `transport` provides serial UDP exchanges, echoed-counter matching, bounded waits, cancellation and strict response lengths. Failed exchanges poison the session; UPLOAD is not repeated on an uncertain MTA. Acquisition recovery operates above this transport by opening a fresh session and restarting a whole record. Address values are opaque selectors, so byte alignment is enforced on transfer lengths rather than invented address arithmetic.
- `log_info` parses bounded plain records, permissions and lazy sector plans. A==B is empty; B>A selects [A,B); wrapped logs select [0,B), then [A,N+1), with each range traversed newest first. Malformed mappings, unsupported address granularity/lengths and sector overflow fail explicitly.
- `discovery` uses native-only `mdns-sd 0.21.4`, preserves IPv6 scope/interface information and raw TXT values, and shuts down its daemon after success, cancellation or error. Unscoped link-local endpoints are unusable. No discovered ECU is automatically probed.
- `capture` limits info/config/sector/aggregate bytes and record/sector counts before payload requests and allocation. Defaults: 4 MiB info, 64 MiB config, 16 MiB sector, 2 GiB aggregate, 10,000 sectors and 4,096 parsed records. These are client limits, not recovered hardware maxima.
- After initial inspection succeeds, transient read timeouts and selected temporary socket errors trigger up to three reconnect attempts across the **whole capture**, with cancellable 100/200/400 ms backoff. `CaptureLimits.max_reconnects` and CLI `--reconnect-attempts` allow 0–8; zero disables recovery. No retry budget is reset per record. Initial connection/inspection failures remain explicit errors.
- Recovery retires the uncertain session before connecting and keeps old UDP sockets bound, including failed handshake sockets, so their local ports cannot be reused by replacement sessions. Old socket destruction cannot disconnect the replacement. The new connection rechecks download rights, compares complete initial logging-info/permission bytes, and re-reads **all previously committed config and sector records** against stored SHA-256 digests. This intentionally conservative check may refuse a legitimate source whose opaque information changes. It establishes observed byte equality, not authenticated ECU identity or an atomic hardware snapshot.
- Only after validation succeeds is the interrupted record restarted with its original SET_MTA selector, header and full payload. Partial attempts are discarded, completed output files are never appended or rewritten, and selectors are never incremented by a guessed byte offset. Changed source bytes stop with an inconsistent bundle; permission refusals, explicit ECU errors, malformed protocol data, cancellation, disk errors and exhausted limits stop without automatic retry. Actual ECU interruption/reconnect behavior still needs hardware validation.
- Recovery verification and every requested UPLOAD byte, including requests with lost replies, count toward the same aggregate transfer budget. Progress reports transfer work, while manifest `total_bytes` reports persisted file bytes; they can differ after recovery. Recovery decisions are retained in manifest diagnostics and the UI/CLI reports reconnection while remaining cancellable. A successful final DISCONNECT is not required to retain completed data.
- Output files use a new unique directory and `create_new`; existing captures are not overwritten. Manifest updates serialize and sync a sibling before replacement. Interrupted or failed transfers retain an incomplete bundle and expose its path. Changed selected boundaries produce an inconsistent bundle. Equal boundaries establish only that comparison, not an atomic or checksum-validated snapshot.
- The GUI uses a dedicated cancellable worker and bounded message channel. Closing the window cancels work; job polling continues while closed. It does not block the shared graph/session worker or load raw bundles as recordings. Completed LD recordings can be opened through the normal recording loader.
- Native image/catalog/telemetry parsing and fresh LD serialization impose byte, record, XML depth/text, channel and sample limits before large allocations. Output grids are capped at 20 million samples. Conversion persists incomplete manifests on cancellation or failure and never overwrites an existing capture or recording directory.

## Validation and remaining work

Workspace tests pass, including synthetic codec, record, sector-plan, discovery-model and UDP mock-device coverage. Seven recovery tests inject lost replies after MTA advancement, late packets, lost reconnect handshakes, failures during source verification, failures across different records, final-info interruption, permission revocation, changed boundaries/config/committed sectors, malformed/negative replies, exhaustion, cancellation and byte-budget pressure. M1-specific additions cover stale counters/events, negotiated endianness and granularity, deadlines/cancellation, permission refusal, byte/count caps before payload reads, exact raw persistence, partial bundles and changed boundaries. The independent review also checked output replacement and GUI worker lifecycle. These fixtures are generated examples, not captured ECU traffic.

Remaining acceptance work:

1. Broaden physical validation to additional M1 hardware and firmware, including reconnects after real interruptions and sustained throughput. Devices requiring credentials/keys remain unsupported.
2. Broaden native conversion validation across additional images and Tune exports, including nondefault clocks, multiple catalogs, protected-image/permission behavior and firmware variation. Validate i3rs extended LD interoperability in other applications before claiming i2/Tune compatibility.
3. Broaden native GUI acceptance on Windows and Linux. macOS acceptance completed after the user unlocked the Mac.

No support for other MoTeC device families or complete i2 acquisition parity is implied.

Completed local checks after the final conversion additions:

- `cargo test -p i3rs-core --lib m1::` with both local numeric-image and XML fixture paths — 110 passed, 2 explicitly ignored local-fixture tests; includes native conversion and LD reopening. Real fixture bytes remain ignored.
- Fresh LD writer: seven focused tests cover multiple frequencies, phase handling, missing gaps, float64/u32 precision, full names, signed enums, allocation limits and invalid timestamps.
- Catalog: eight focused tests cover aliases, Unicode enums, explicit affine units, unknown-unit fallback warnings, structure and budgets.

- `cargo test --workspace` — passed, including 272 core unit tests, 155 app tests and 11 M1 CLI integration tests (existing fixture-dependent ignores retained).
- `cargo test -p i3rs-core m1::capture::tests` — 9 passed after the manifest review fix.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo check -p i3rs-app --target wasm32-unknown-unknown` — passed.
- `cargo check -p i3rs-core --target aarch64-pc-windows-msvc` — passed; compile check only, not Windows runtime validation.
- `cargo build --release`, formatting and diff checks — passed.
- Native GUI: File → Download M1 logs, destination selection and saved-image selection converted the numeric image to four completed LDs. The first opened through its result button; the second independently reopened through the normal macOS file dialog. Telemetry graphs and state labels rendered. Earlier checks verified scan completion/cancellation, scope validation and connection-refused recovery.
- `bash scripts/build-macos-dev.sh` — built and verified `~/Applications/i3rs Dev.app` with its existing `i3rs Development` certificate and designated requirement.

REA exported 76 focused evidence records. Provider shutdown reported that sandbox process inspection prevented cleanup confirmation; a subsequent approved process inspection confirmed the owned Ghidra and temporary client had exited, and their isolated temporary directory was removed after retaining diagnostics.

### Recovery acceptance (2026-10-27)

The reconnect addition passed the full workspace suite (272 core unit tests, existing ignores retained), Clippy with warnings denied, release builds, the Windows ARM64 core check and the web app check. Fault injection covers seven test cases with multiple failure modes. The low-level transport still never retries an uncertain UPLOAD.

An additional loopback acceptance run served the real numeric fixture's setup, metadata and 14 sectors behind synthetic info/permissions. It dropped a sector reply after advancing MTA, then delivered that reply late to the retired socket. CLI `m1 download` recovered on a different local port, completed acquisition and converted four LDs. All 16 saved configuration/sector files matched their fixture sources byte for byte. This validates software recovery and conversion against a mock; it does not establish physical ECU reconnect semantics. The temporary loopback server is stopped.

After the user unlocked the Mac, the previous Dev instance was quit through Computer Use and `scripts/build-macos-dev.sh` refreshed the fixed Dev app with the unchanged certificate/designated requirement. Native GUI inspection and download against the loopback fixture completed after an injected lost UPLOAD reply and a delayed reconnect response. Manifest diagnostics confirm one verified recovery; all 16 saved config/sector files matched the fixture sources exactly. Four converted recordings were offered, and segment 2 independently reopened through the normal macOS file dialog with telemetry graphs and the original missing interval visible. The temporary mock server was stopped. This is GUI/software acceptance, not a physical ECU test.

### Fast Upload

A real ECU trial acquired 990 of 992 sectors at about 225 kB/s before a generic XCP rejection. The operator observed a peak of 2,500 kB/s on the same ECU and connection with M1 Tune. The ECU was unavailable for this follow-up.

The official M1 Tune 1.5.0.0439 installer was unpacked offline. The x64 executable SHA-256 is `ef82831a25458988d30faa4cd7da07fdd59a4984f9cefa0aac910366f42e7ddf`. Its Fast Upload request constructor (`0x140913e30`) and submit method (`0x1409170e0`) show an eight-byte XCP user command beginning `F1 FF`, with a negotiated-endian byte count at offset 4. The response index reader (`0x140919d20`) and copier (`0x14091a660`) use a negotiated-endian u16 index at offset 2 and place bytes after the four-byte reply prefix at `index * (maxDTO - 4)`. In 1.4.0.0302, `0x777ab0` and `0x779c90` show the same count and indexed copy. Its dispatcher (`0xe58040`) matches the request's counter for FF/FE responses and keeps the command pending for subsequent packets. The REA decompile timed out on the 1.5 executable, so these observations are from bounded LLVM disassembly and RTTI cross-references.

i3rs checks permissions and reads each record's fixed header over standard XCP. It enables Fast Upload only after a multi-packet read matches the same complete record from standard XCP byte for byte. The receiver validates request counters, chunk indices, chunk lengths and duplicate contents before committing a record. It caps the block at 512 KiB and charges the full request to the existing transfer budget. Lost chunks restart the entire record on a fresh session after source verification. Unsupported or invalid Fast Upload replies can trigger a verified return to standard XCP. The CLI exposes `--standard-upload` for a direct comparison, and the manifest records negotiated sizes, mode verification and acquisition throughput.

A loopback server used real ECU sector bytes and a synthetic 1 ms delay per request. Over 16 sectors (4,153,412 bytes), the CLI measured 5,116 kB/s with Fast Upload and 174 kB/s with standard UPLOAD, including startup and disk writes. All sector files matched the input in both modes. These figures establish client behavior under that model. Actual peak and sustained ECU performance require a hardware run.

The signed macOS Dev app also completed a loopback download through **Download and convert logs**. Its manifest records verified Fast Upload, all 14 sectors and both configuration records matched the source bytes, and four converted recordings were available in the app. The first reopened through the result button.

### M150 validation (2026-11-24)

i3rs captured all 992 reported sectors and converted them into 22 recordings. Verified Fast Upload averaged 1,609 kB/s across the full transfer. Tune showed 2,440.1 KB/s during a later attempt and retained 238 sectors. All 237 closed sectors in that image matched i3rs byte for byte. The newest sector had a matching payload prefix followed by newly logged bytes.

All 207 shared channels matched by name, units and sample rate. Tune also exported an internal beacon channel. i3rs retains ECU elapsed time and splits recordings at resets, so file boundaries and sample grids can differ. The original Tune image now converts directly into six recordings. Its 43 leading numeric packets are CRC-checked and reported when skipped because the setup and clock context is missing. The conversion masks the two observed invalid Traction Aim payloads and avoids interpolating GPS coordinates across transitions to or from the no-fix pair. These fixes removed 838 invalid values and three spurious coordinates from the saved capture.
