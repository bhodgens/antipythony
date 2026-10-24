# LICENSES/ - license texts for code we incorporate

- MIT-pon.txt: exact LICENSE text from can1357/pon (pon-vm, Can Bölük,
  me@can.ac), copied from the license-pinned clone (rev d0a17177) used for
  the 01.03/02.05 study (docs/pon-study-lowering.md). pon is dual-licensed
  MIT/Apache-2.0; the full code-donor grant (adapt/vendor freely with
  attribution) was given by email, 2026-10. Apache-2.0.txt is the standard
  Apache-2.0 text for that second option.
- pon-license-grant-email.txt: the primary evidence for that grant -
  Benjamin Hodgens' licensing request to Can Bölük (me@can.ac /
  can.boluk89@gmail.com) and Can's reply "Sounds awesome. Done!" (Fri,
  16 Oct 2026), full MIME thread with message-ids and DKIM headers,
  as saved by the project owner. Commit status as of this file's landing:
  no pon-derived code exists in crates/ yet; design studies are in
  docs/pon-study-lowering.md and docs/pon-study-tier1.md. Adapted files
  must carry the attribution header ("adapted from pon, Copyright Can
  Bölük, MIT/Apache-2.0") per master.md before their first commit.
  NOTE: this email contains the sender's personal phone number; keep it
  because it is the license-evidence provenance the owner directed to
  archive (owner instruction 2026-10-23).
- Gate (AGENTS.md): BEFORE the first pon-derived commit, these entries plus
  attribution headers on adapted files are required. Landed pre-emptively
  (bughunt 2026-10-17, auditor-1) so Plan 02 adaptation cannot ship
  without them.
- PSF-LICENSE-2.0.txt + README-vendored-stdlib.md: CPython stdlib files
  under vendored_stdlib/ (see that README).
