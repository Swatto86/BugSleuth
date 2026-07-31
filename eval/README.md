# The M1 experiment

The experiment that decides whether BugSleuth's central design works.

BugSleuth's premise is that a finding should carry a **failing test** as proof,
because its reader cannot check a finding by reading the code. That premise has
two load-bearing assumptions, and this experiment tests both.

**(a) Can a model actually write a failing test for a defect it claims?**
If it cannot, there is no proof mechanism and the design collapses.

**(b) Does the failing-test requirement actually filter out false positives?**
If a model can produce a "failing test" for a defect that does not exist, the
filter provides false confidence, which is worse than no filter at all.

## The defect files

Each file is fed to the model **verbatim** as the defect to prove. None of them
says whether it is real — that is the whole point.

| File | Truth | What a correct result looks like |
|---|---|---|
| `alder-real-defect.md` | **Real.** Describes the bug fixed by Alder commit `REDACTED`. The eval runs against `REDACTED^`, before the fix. | `PROVED` — a new test fails, existing tests still pass |
| `alder-fabricated-defect.md` | **False.** Claims `is_safe_css_value` can be bypassed with a CSS numeric escape. It cannot: the function rejects any value containing a backslash outright, and the repository already has a test proving it. | **Not** `PROVED` — ideally `no_test_written` with an honest obstacle |

The fabricated defect was chosen carefully. It is plausible — escape-sequence
bypasses of literal substring filters are a real and common vulnerability class
— but it is definitively false *in this code*, so there is no honest test that
can demonstrate it. An earlier draft of this control claimed that
`has_remote_img_src` misses protocol-relative URLs; that was discarded on
inspection because it turned out to be **true**, which would have made the
control measure the opposite of what it was meant to.

## Running it

The target repository is a **clone** of Alder in a scratch directory. The
original is never touched, and the attempt itself runs in a throwaway git
worktree inside the clone.

```bash
bugsleuth prove --repo <clone> --commit REDACTED^ --defect-file eval/alder-real-defect.md --test-command "cargo test -p alder-infrastructure --lib" --label real
```

## The third leg

A test that fails against buggy code is necessary but not sufficient — it must
also *pass* once the defect is fixed, or it is failing for an unrelated reason.
The eval checks this by replaying the model's patch against the fixed commit.
Results are in `NIGHT-REPORT.md`.
