# Security Policy

Mnema records screens and audio and stores them encrypted on-device, so security and privacy reports are taken seriously.

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Instead:

- Use GitHub's [private vulnerability reporting](https://github.com/shaik-zeeshan/mnema/security/advisories/new), or
- Email **shaikzeeshan999@gmail.com** with details and reproduction steps.

You'll get an acknowledgment within a few days. Please give a reasonable window to ship a fix before public disclosure.

## Scope

Of particular interest:

- Anything that lets captured data (recordings, OCR text, transcripts, database contents) leave the device without explicit consent
- Encryption weaknesses in the local database or secret vault
- Privacy-exclusion failures (excluded apps still being captured on screen or in system audio)
- License-verification bypasses are out of scope for public reports — but feel free to email

## Supported versions

Only the latest release receives security fixes.
