# MP3 frame header import

`header.rs` and `common.rs` are imported from Symphonia bundle-mp3 0.6.1,
revision ee35874b571a35a9a6e15d3bc9a3aaf8f11fbeee. They retain their original
copyright and MPL-2.0 file licenses. LICENSE contains the complete license;
PROVENANCE.json binds the exact original and ported bytes. Only import paths
and formatting differ. No DSP or decoder is forked here.

The surrounding mpeg.rs adapter is MIT. It uses the already enabled Symphonia
metadata reader and imported header parser to measure contiguous Layer 3
frames, including Xing/Info metadata frames. Leading ID3v2 and trailing ID3v1
are supported; discontinuities and unsupported trailing bytes refuse. This
physical index does not infer gapless timing, native cached seek tables or
player compatibility. Existing export paths do not use the new module.
