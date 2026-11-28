# AppImage runtime components

The AppImage contains the unmodified AppImage type2-runtime release `20251108`. Its own license and notices for musl, libfuse, squashfuse, zstd, zlib and mimalloc are included in the application bundle. These terms apply to those components; Stutter Watch retains its root license.

`upstream.json` pins the downloaded runtime source, the libfuse and squashfuse source archives, and all copied notices by SHA256. The libfuse and squashfuse archive hashes also match the runtime's upstream build recipe.

The release's `appimage-runtime-sources.tar.gz` contains these sources, the runtime's libfuse patch, its Dockerfile and build/relink instructions, and the notices. Unpack the runtime source archive and follow its `BUILD.md` to rebuild the standalone runtime with a modified library. The upstream recipe uses Alpine 3.21 for its remaining native dependencies.

The packaging script does not execute downloaded source archives. It verifies each checksum before including the source distribution.
