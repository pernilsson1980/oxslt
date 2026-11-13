# Security

oxslt treats every stylesheet and document as untrusted input: it has no
`unsafe` code and no network access, and it limits recursion, entity
expansion and the sizes libxml2 limits. The README's Security section says
what it reads and writes.

## Reporting a vulnerability

Please report vulnerabilities privately, through GitHub's
[private vulnerability reporting](https://github.com/pernilsson1980/oxslt/security/advisories/new),
not in a public issue. Include a stylesheet and input that show the problem
if you can.

Crashes, hangs and runaway memory use on hostile input count as security
issues. A difference in output from `xsltproc` is an ordinary bug: please
open an issue.
