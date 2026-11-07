# Rein concept explainer: narration script

Source of truth: `narration/beats.json`. `scripts/tts.py` speaks each `speech` line with Kokoro v1.0 (voice `am_michael`). `captions` on screen come from the `caption` field.

| # | Beat | Narration (spoken) | On-screen caption |
|---|------|--------------------|-------------------|
| 1 | Hook | Agent frameworks make you build the machine: graphs, nodes, edges, state, retries. | Frameworks make you build the machine |
| 2 | Turn | Rein flips it. You don't wire the agent. You tell it what to do. | Rein flips it. You say what to do. |
| 3 | Function = file | Every decision is a small function, and every function is its own file. Each one starts with a description of what it decides. | One decision, one function, one file |
| 4 | Inputs = questions | A function's inputs are questions. The agent answers them from what it sees: yes or no, or one of a few options. | Inputs are questions the agent answers |
| 5 | Body = if / switch | Inside, there are only two moves: if, and switch. No loops, by design, for now. | Only two moves: if, and switch |
| 6 | Return | Every path ends in one of two things: a plain instruction for the agent, or a pointer to the next function. | Every path ends in one of two things |
| 7 | Flow | The agent starts at the project's main function. It answers only the questions on its path, and follows pointers until it gets its instruction. | Start at main. Answer. Follow the pointers. |
| 8 | Safety + tooling | A checker makes sure every path returns, and every pointer is real. The editor shows it as you type. | Checked as you type |
| 9 | Close | Rein. Instructions, not infrastructure. Alpha, coming soon. | Instructions, not infrastructure. |

Lengths are set per beat by the measured narration plus 0.4 s lead and 0.8 s tail. Total is about 68.5 s.
