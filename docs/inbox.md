# Inbox

<!--
- [ ] Add dark mode
- [ ] Fix the sign-in error shown after a session expires
- [ ] Decide whether exported reports should include archived records
-->

- Replace setup audio-persistence error-string classification with typed
  internal errors before attempting a broader setup-backend refactor; the
  current branches are tested and stable, while changing them would widen the
  hardware-sensitive transaction surface without a release benefit.
- Evaluate additional systemd sandbox directives such as address-family and
  filesystem restrictions in an isolated live compatibility matrix; retain
  only hardening proven compatible with BlueZ and desktop notification D-Bus.
- Clarify whether the five-second lifecycle reconciliation interval is a strict
  wall-clock deadline during in-flight ANCS attribute work. The current bounded
  fairness fix reconciles between notification units, while one notification
  plus an uncached app-name lookup can consume two five-second request windows;
  a strict deadline would require cancellation-safe interleaving and focused
  hardware validation.
