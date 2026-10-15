# Security Summary page — spec (from Dickles' 2019 wishlist, green-lit 2026-10-14)

Dickles asked for this twice in his old Linux wishlist sheets (the
"security summary" line in the Washing Machine tab and the "security
checkup checklist" in Top Software): one page that answers, in plain
English, *is this machine set up safely?* It is a natural Profiler
page — the Profiler already knows the hardware, OS, and update state;
this page renders verdicts, not raw data.

## The page

A list of checks, each with a plain verdict — **Good** / **Worth a
look** / **Off** — one line of what it means, and where relevant a
button that takes the user to the thing that fixes it (never a fix
applied from this page; the Profiler's check-only law stands — apply
actions stay in the Updates hub behind their own polkit prompts).

Checks for v1:
- **Disk encryption** — is the root volume LUKS-encrypted (lsblk /
  cryptsetup state). Verdict wording matters: "Your drive is not
  encrypted — anyone who steals this PC can read every file."
- **Firewall** — ufw / firewalld active or not.
- **Backups** — a backup tool configured (Timeshift, Déjà Dup,
  Borgmatic, restic profile) and when the last successful run was,
  if discoverable. "No backup found" is a red verdict.
- **Security updates waiting** — count from the existing Updates hub
  data, security-tagged updates called out separately.
- **Screen lock** — lock on suspend / idle lock enabled (gsettings
  or the DE equivalent), lock delay.
- **Camera & microphone** — any app currently using them (PipeWire
  nodes), plus whether mic is muted at the system level.
- **Remote access** — SSH server installed/running, RDP/VNC servers
  listening. Running-and-exposed is "Worth a look", not red.
- **Guest / extra accounts** — guest session enabled, accounts with
  no password, autologin enabled (autologin on an unencrypted disk
  is the combination verdict worth surfacing).
- **Secure Boot** — on/off (mokutil), stated factually.

## Rules

- Read-only. No check shells out with privileges; anything that
  needs root to inspect is reported as "couldn't check" rather than
  prompting. Prompts belong to actions, not to looking.
- Every verdict line is written for a non-technical reader first;
  the technical detail (device, service name, raw value) sits in an
  expandable row under it — the Profiler's usual two-level pattern.
- The page re-checks on open and on demand (↻). No daemon, no
  scheduled nagging.
- Summary header: "n of m checks look good" — never a fake
  percentage "security score". Scores imply precision we don't have.

## Placement in the build queue

Profiler next build, alongside the already-owed items: the
in-family icon redraw (ICON-FAMILY.md) and the Report a Bug /
Request a Feature entries.
