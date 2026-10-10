# Maintaining Matinee

Notes for the project owner: getting updates onto GitHub and publishing releases.

## Applying an update

Updates arrive as `matinee.tar.gz`. They never contain `README.md` or a `.git`
folder, so your README edits and your history are safe.

1. From the folder that contains your `matinee` folder, unpack over it:
   `tar xf ~/Downloads/matinee.tar.gz`
2. Inside `matinee`, push it: `./sync-to-github.sh "Describe the update"`

The sync script first brings down anything you changed on the GitHub website,
puts your update on top, then pushes.

## Releases

GitHub builds and tests every push; follow it in the repository's **Actions**
tab. When the version in `Cargo.toml` hasn't been released yet, the same run
publishes a release with a ready-to-install archive. To release a new version:

    ./release.sh 0.4.0

That updates the version in `Cargo.toml` and `Cargo.lock`, commits and pushes.
A few minutes later the release appears under **Releases**, with install steps
for your friends and a list of what changed. Pull requests are built and tested
but never released, and pushing a `v0.4.0` tag works too.

The first push of the workflow needs GitHub's `workflow` permission. The sync
and publish scripts ask for it once: a browser window opens; approve it.

## Files

| File | What it does |
|---|---|
| `.github/workflows/release.yml` | builds, tests and publishes releases on GitHub |
| `.github/release-notes.md` | the install steps shown on every release |
| `scripts/package.sh` | builds the release archive (works locally too) |
| `release.sh`, `sync-to-github.sh`, `publish-to-github.sh` | the commands above |
