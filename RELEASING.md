# Releasing shrinkfit

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`, which:

1. checks that the tag matches every package version (`scripts/version.py`);
2. builds a Python wheel for each platform (Linux x86_64/aarch64 in manylinux
   containers, macOS arm64/x86_64, Windows x86_64), installs each and runs the
   Python tests; builds, type-checks and tests the npm package; dry-runs
   `cargo publish`;
3. only if all of that passed, publishes **crates.io** (`shrinkfit`, then
   `shrinkfit-cli`), **PyPI** and **npm** in parallel.

Branches named `release-test/*`, pull requests touching the workflow, and manual
runs do steps 1 and 2 but never publish.

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

### npm

1. Create an **automation** (or granular, publish-scoped) access token at
   <https://www.npmjs.com/settings/~/tokens>.
2. In GitHub, create the environment `npm` and add the secret `NPM_TOKEN`.
   (Provenance statements are attached automatically.)

Tip: on each environment add a **required reviewer** (yourself). Every publish
then waits for a manual approval click, a cheap safety net.

## Cutting a release

```sh
python scripts/version.py 0.2.0     # bumps all five version fields
cargo update -w                     # refreshes Cargo.lock
git commit -am "Release 0.2.0" && git push
git tag v0.2.0 && git push origin v0.2.0
```

Watch the run under the repo's **Actions** tab. To rehearse without publishing:
`git push origin HEAD:refs/heads/release-test/try1`.

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
