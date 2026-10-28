# Signing commits and tags

_Last edited: 2026-10-27_

Release **files** are already signed (minisign, see `docs/workflow.md` and `docs/update.md`). This is about the **source history**: signing commits and tags with an SSH key so GitHub shows a green **Verified** badge, and anyone can check that a commit or release tag came from the maintainer and not from someone who got hold of a GitHub token.

## Where things stand (2026-10-27)

* No signing is configured on the maintainer's machine (`git config` has no `gpg.format`, `user.signingkey`, `commit.gpgsign` or `tag.gpgsign`), so commits pushed from it are **unsigned**.
* Merge commits made by `gh pr merge --merge` or the GitHub merge button are created and **signed by GitHub** (they show Verified on GitHub). Locally `git log --show-signature` shows them as unverifiable only because your git doesn't have GitHub's public key.
* Release tags (`git tag vX.Y.Z`) are unsigned.
* `main` has **no** "require signed commits" rule. Do not turn it on until step 4 below is done, or every unsigned commit on a pull request branch will block its merge.

## 1. Make a signing key

A dedicated key, not your login key, so it can be revoked on its own:

```sh
ssh-keygen -t ed25519 -C "opensanctuary signing" -f ~/.ssh/opensanctuary_signing
```

Give it a passphrase, and add it to the agent (`ssh-add ~/.ssh/opensanctuary_signing`) so it is asked for once per session. A hardware key (`ssh-keygen -t ed25519-sk`) works the same way and keeps the private key off the disk.

## 2. Tell git to use it

```sh
git config --global gpg.format ssh
git config --global user.signingkey ~/.ssh/opensanctuary_signing.pub
git config --global commit.gpgsign true     # sign every commit
git config --global tag.gpgsign true        # sign every annotated tag (a lightweight `git tag name` cannot be signed)
```

(Use `--local` instead of `--global` to limit this to this repository.) The e-mail in `git config user.email` must be one that is verified on the GitHub account, or GitHub will not mark the commit Verified.

## 3. Tell GitHub about the key

GitHub → **Settings** → **SSH and GPG keys** → **New SSH key** → *Key type*: **Signing Key** → paste `~/.ssh/opensanctuary_signing.pub`. (A key added as an authentication key does not make commits Verified; it has to be added again as a signing key.)

Check it: make a commit, push, and look for the **Verified** badge on it. Locally, `git log --show-signature -1` needs an allowed-signers file to say "Good signature":

```sh
echo "$(git config user.email) $(cat ~/.ssh/opensanctuary_signing.pub)" >> ~/.config/git/allowed_signers
git config --global gpg.ssh.allowedSignersFile ~/.config/git/allowed_signers
```

## 4. Sign the release tag

With `tag.gpgsign` on, the release step in `docs/workflow.md` is unchanged:

```sh
git tag -a v0.4.1 -m "OpenSanctuary 0.4.1"   # signed because of tag.gpgsign
git push origin v0.4.1
git tag -v v0.4.1                            # prints "Good signature" (needs the allowed-signers file)
```

Without the config, use `git tag -s -a v0.4.1 -m "..."`. The release workflow only checks that the tag equals `v<Cargo.toml version>`; it does not read the signature, so signing a tag costs the pipeline nothing.

## 5. Optionally, make GitHub enforce it

Once every way commits reach `main` is signed, add the rule: repository **Settings** → **Rules** (or **Branches**) → `main` → **Require signed commits**. Commits that reach `main` this way are fine:

* a pull request merged with the **Merge** button or `gh pr merge --merge` (GitHub signs the merge commit); but the **branch's own commits** must be signed too, so every commit you or a tool pushes to the pull request branch needs the key;
* a version-bump merged with `gh pr merge --admin` is the same: the commits on its branch must be signed.

Anything that pushes commits for you (an editor, a script, an AI assistant working in your checkout) signs with your key only if it runs where the key is loaded (the SSH agent) and git is configured as above. If that is not the case for some tool, either do not enable the rule yet or let that tool open pull requests whose commits you re-create signed (`git rebase --exec 'git commit --amend --no-edit -S' main`).

## Limits worth knowing

* Signing proves who made a commit, not that the code is good. It does not replace review or CI.
* **Verified** on GitHub means the signature matches a key registered to that account; if the key is lost or stolen, remove it from the account (commits signed with it then stop showing Verified) and add a new one. Old tags stay as they are.
* CI cannot sign as the maintainer: the workflows have no access to the private key, and should not. Release files are signed by the separate minisign key kept as a GitHub secret.
