# Upstream notices omitted from registry packages

Some dependencies declare their license in Cargo metadata but omit the full text from their published crate. These copies fill those gaps for binary distribution. `upstream.json` records the exact upstream URL, commit and SHA256 for every copied file. Packaging verifies those hashes and includes notices only for dependencies selected for the target build.

The egui 0.37 notices come from the official 0.37.0 release tag, commit `841a328ba2dd8f6996beb7189ebb7fc00e668a2a`. The unpublished source commit IDs recorded in some crate archives differ; both identities are retained in the manifest. Font notices are also copied directly from the published font crate.

These are third-party terms. Stutter Watch's own license is the repository's root `LICENSE`.
