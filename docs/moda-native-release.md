# Moda Windows native releases

The fork's main branch tracks a stable upstream Skia Canvas version and Moda's
Windows offscreen backend changes. Studio uses the upstream JavaScript package
and replaces the Windows x64 native addon during packaging.

For an upstream upgrade, branch from the new upstream stable tag and migrate
the Windows backend commits by feature. Verify backend selection and fallback
(Vulkan, OpenGL, CPU), bitmap readback and loading from an Electron package.
Keep the upstream version in Cargo.toml and package.json aligned with Studio.

Tag a verified main revision as `v<upstream-version>-moda.<revision>`. The tag
workflow publishes the Windows x64 addon and a JSON manifest with the source
commit and SHA-256 of the compressed and uncompressed bytes. Studio can pin
that tag and the uncompressed digest in its packaging configuration.
