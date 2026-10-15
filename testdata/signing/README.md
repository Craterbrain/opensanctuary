# Test signing keys

**These keys are public and worthless.** They are throwaway minisign key pairs (the secret keys have no password and are
committed on purpose) so that the plugin-signing tests and manual experiments can sign things without the real release
secret. Nothing signed with them is trusted by a normal OpenSanctuary: the app only honors `release.pub` as its plugin
release key when it is started with the hidden `--test-release-key` flag, and it logs a loud warning when it is.

| Key | Stands in for |
|---|---|
| `release` | The OpenSanctuary release key (signs the revocation list and the bundled plugins) |
| `acme` | A third-party publisher |
| `rival` | A second third-party publisher |

Try it by hand:

```sh
# start the app so it treats the test key as its release key
os-next --test-release-key "$(sed -n 2p testdata/signing/release.pub)"

# sign a plugin as "acme": list the files, set publisher_key, sign the manifest
os-next --hash-plugin my_plugin
#   (edit my_plugin/plugin.json: "publisher_key": "<second line of acme.pub>")
minisign -S -s testdata/signing/acme.key -m my_plugin/plugin.json -x my_plugin/plugin.json.minisig

# or sign the whole folder in one step with the helper the release process uses
cargo run --release --example sign_release -- sign-plugin testdata/signing/acme.key my_plugin

# a revocation list signed by the test release key
echo '{"sequence":1,"revoked":[{"key":"<rival.pub key>","reason":"test","since":"2026-10-14"}]}' > web/revoked_keys.json
minisign -S -s testdata/signing/release.key -m web/revoked_keys.json -x web/revoked_keys.json.minisig
```

Regenerate them (rarely needed) with `cargo run --release --example gen_test_keys -- testdata/signing release acme rival`; the files are plain minisign keys, so the `minisign` command-line tool reads them too.

The automated tests read these files (`tests/core_tests.rs`, `web/tests/e2e_plugin_revocation.test.ts`).
