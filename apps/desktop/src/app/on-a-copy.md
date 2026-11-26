---
name: on-a-copy
description: Runs a coding task on a throwaway cloud machine from t3agent37, with this project's checkout as it is here (branch, commits not pushed, changes not committed), then brings the changes back into this checkout. Use when the user says "on a copy", "/on-a-copy", "in the cloud" or "in a sandbox", or asks to send, offload or hand off work to a copy.
---

# On a copy

Written by t3agent37 (Settings → "Agents may send work to copies"); turning that off removes it.

The user wants this task done on a machine of its own: a cloud machine made for this task and deleted after, either new (signed in with what they saved in t3agent37) or a copy of their base (with its sign-ins and tools), as they chose there. Run it from here with t3agent37's tools (`spawn_machine`, `spawn_status`, `collect_machine`) and T3 Code's `delegate_task`. They asked for it, so don't ask again whether to. It bills a few cents an hour while it's up.

1. **Spawn.** Call `spawn_machine` with `path` set to this project's checkout: your working folder, a git repository. Add `size` only if the user named one, and `base` only if they asked for a copy of a base (or `"new"` for a new machine). The machine gets the checkout exactly as it is here, and the project is cloned and set up there. The call answers before the machine is ready.
2. **Wait.** Tell the user a machine is being made: a minute or two, several minutes for a copy of a base. Then call `spawn_status` until it says the machine is ready; each call waits about half a minute. If it failed, say why and stop.
3. **Hand it the work.** Call `delegate_task` with the `target` that `spawn_status` gave (`{environmentId, projectId}`) and a `task` that stands on its own, since the agent there can't see this conversation. Give it the user's request in full and what you know that matters (files, errors, constraints), and tell it to work in the project's checkout as it is, keep its uncommitted changes, commit only if the user asked for commits, and never push. Then give the user the thread link `delegate_task` returned: in T3 Code it opens that machine's thread, so they can watch it work (read-only, with T3 Code's "Connect to interact" to step in). Its result comes back to this thread by itself: don't poll for it, and don't change the same files here meanwhile.
4. **Bring it back.** When the result arrives, tell the user what was done. Then call `collect_machine` with the copy's name. Its changes are made to the files here, uncommitted, when they still fit them, and the copy is deleted. t3agent37 also keeps the copy's work on a branch of its own here, named after the copy (`collect_machine` says which): a record it makes, not a commit on the user's branch, so mention it, and that they can delete it. If the changes don't fit, because these files changed meanwhile, say the work is on that branch.
5. For a follow-up before collecting, call `delegate_task` with the same target again. For several separate tasks, spawn a copy for each.
6. If the user wants to step in and write to the copy's agent themselves, call `add_to_t3` with the copy's name: T3 Code asks them to add the copy, and the thread link then opens a thread they can write in. When you collect that copy, tell them to remove it in T3 Code too (Settings → Environments).

If `spawn_machine` says agents may not send work to copies, that new machines have no agent to work with, or that there's no base, tell the user what it says: they set those up in t3agent37. Don't work around it. Outside T3 Code, without `delegate_task`, say that this needs a thread in T3 Code.
