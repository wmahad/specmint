# Security policy

## Supported versions

Only the latest release of specmint gets security fixes.

## Reporting a problem

Please don't open a public issue. Report it privately through GitHub instead: open the [Security tab](https://github.com/wmahad/specmint/security) and choose "Report a vulnerability".

Include what you found, how to reproduce it, and which version you used. You'll get a reply within a week. Once a fix is released, the report is published as a security advisory, crediting you unless you'd rather stay anonymous.

## Scope

specmint downloads specs from URLs you configure and writes files to the output folder you configure. Problems worth reporting include writing outside the output folder, leaking credentials such as `SPECMINT_BASIC_AUTH`, or generated code that is unsafe to run.
