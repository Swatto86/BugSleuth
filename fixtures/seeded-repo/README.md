# seeded-repo

Test data for BugSleuth. **Every defect in here is deliberate.** Do not "fix" it.

A tiny inventory service with a small number of blatant, independently
discoverable bugs. It exists so a sweep has something it must find: if BugSleuth
reports a clean bill of health on this repository, BugSleuth is broken.

The seeded defects are listed in `SEEDED.md`, which is deliberately **not**
readable by a sweep — see `fixtures/README.md` for how the sweep is scoped.
