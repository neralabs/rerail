---
name: install
description: Install Rerail in Codex and set it up in the current repository. Use when the person asks to install, set up or try Rerail.
---

# Install Rerail

Rerail installs with its own set-up, run through `npx` at the version this plugin pins. Set-up asks the person
questions; your job is to relay them and run the command for each answer. Never answer a question for the person.

1. Check that this is Codex with a terminal you can run commands in. If it is not (ChatGPT chat),
   tell the person to run `npx @neralabs/rerail@0.1.0 setup` in a terminal, in their repository, and stop.
2. Check `node --version` is 24 or later and `git --version` works. If not, say which is missing and stop.
3. From the repository's root folder, run:

   ```sh
   npx --yes @neralabs/rerail@0.1.0 setup --relay
   ```

4. Set-up prints one question at a time, with one command per answer, and exits. Show the person the question and its
   answers exactly as printed, wait for their reply, then run the command printed for that answer, exactly as printed,
   putting their words only where the command shows a placeholder such as `<the folder they name>`. Repeat until
   set-up prints no further question.
5. Report set-up's last output as printed. It says what to do next, such as starting a new Codex session and trusting the
   hooks set-up wrote with `/hooks`.

If a command fails, show its output as printed and stop. Rerail's
[troubleshooting](https://github.com/neralabs/rerail/blob/main/docs/troubleshooting.md) covers set-up's errors.
