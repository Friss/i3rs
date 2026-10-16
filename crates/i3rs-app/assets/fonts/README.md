# Japanese UI fallback

`i3rs-ui-jp.ttf` is a static weight-400 subset of Noto Sans JP from
https://github.com/google/fonts/tree/main/ofl/notosansjp, licensed under the
included SIL Open Font License. It contains the Japanese characters used by
application options and help; telemetry channel names retain normal fallback.

Upstream TTF SHA-256: `c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f`.
Generated with fonttools 4.60.1: instantiate weight 400, retain characters at
U+3000 and above from Rust UI source, then rename to I3RS UI Japanese.
Regenerate this subset when adding Japanese translations.
