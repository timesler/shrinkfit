# Releasing shrinkfit

Publishing a **GitHub release** runs `.github/workflows/release.yml`, which:

1. takes the version from the release's tag (`v0.2.0` becomes `0.2.0`) and
   stamps it into every package on the CI runner (`scripts/version.py`), so
   **nothing needs to be committed to cut a release**;
2. builds a Python wheel for each platform (Linux x86_64/aarch64 in manylinux
   containers, macOS arm64/x86_64, Windows x86_64), installs each and runs the
   Python tests; builds, type-checks and tests the npm package; dry-runs
   `cargo publish`;
3. only if all of that passed, publishes **crates.io** (`shrinkfit`, then
   `shrinkfit-cli`), **PyPI** and **npm** in parallel.

Releases marked **pre-release**, branches named `release-test/*`, pull requests
touching the workflow, and manual runs do steps 1 and 2 but never publish.

## One-time setup

### crates.io

1. Create an API token at <https://crates.io/settings/tokens> with scopes
   `publish-new` and `publish-update`.
2. In the GitHub repo: **Settings → Environments → New environment**
   `crates-io`; add the secret `CARGO_REGISTRY_TOKEN`.

### PyPI (trusted publishing, no token)

1. At <https://pypi.org/manage/account/publishing/> add a **pending publisher**:
   project `shrinkfit`, owner `timesler`, repository `shrinkfit`, workflow
   `release.yml`, environment `pypi`.
2. In GitHub, create the environment `pypi`.

### npm (trusted publishing, no token)

npm's trusted-publisher setting is per package, so the package must exist
before it can be configured. Do this once:

1. **First publish, by hand** (from a machine logged in with `npm login`):
   ```sh
   cd bindings/typescript && npm install && npm run build && npm publish --access public
   ```
2. On <https://www.npmjs.com/package/shrinkfit/access> (Settings → Trusted
   Publisher) choose GitHub Actions: owner `timesler`, repository `shrinkfit`,
   workflow `release.yml`, environment `npm`.
3. In GitHub, create the environment `npm`.
4. Optionally set the package's publishing access to "Require two-factor
   authentication and disallow tokens".

From then on the workflow publishes with a short-lived OIDC credential and
attaches provenance automatically. The workflow skips versions that are
already on npm, so tagging the version you published by hand in step 1 still
publishes crates.io and PyPI normally.

Tip: on each environment add a **required reviewer** (yourself). Every publish
then waits for a manual approval click, a cheap safety net.

## Cutting a release

1. On GitHub: **Releases → Draft a new release**.
2. **Choose a tag**: type a new tag `vMAJOR.MINOR.PATCH` (e.g. `v0.2.0`) and
   pick "Create new tag on publish", targeting `main`.
3. Use **Generate release notes**, then **Publish release**.

That is all. Watch the run under the **Actions** tab. To rehearse without
publishing, tick **Set as a pre-release** (use a tag like `v0.2.0-rc1`).

Note that the version numbers inside the repository stay at whatever was last
committed (they are only rewritten on the build machines), so the files on
`main` can lag behind the latest release. Published packages and registry
metadata always carry the release tag's version.

## If something goes wrong

Registries do not allow re-publishing a version, and each job is independent:
if, say, PyPI succeeded and npm failed, fix the cause, bump to the next patch
version, and release again (or re-run only the failed job from the Actions UI,
which is safe because the artifacts are already built).

## Notes

* The Python wheel is `py3-none-<platform>`: one per platform serves every
  Python 3 version, so cibuildwheel's per-interpreter matrix is not needed.
  There is no sdist (the Rust workspace lives outside `bindings/python`);
  platforms without a wheel cannot `pip install` yet.
* `shrinkfit-ffi` is `publish = false`; it ships inside the wheel and npm package.
