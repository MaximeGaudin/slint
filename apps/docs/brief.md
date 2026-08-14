# SLint

 OK, working on this makes me wonder if this is the right approach. It's very very oppionated and probably not fitting for many.

What if instead, we use everything we learn to create the "ESLint for Agentic SKills".

This would be a CLI, following all the patterns that worked for ESLint but apply this to skills.

I doubt we will be able to do everything with static analysis only but ideally, everything we can do with static analysis should be done with static analysis to speed up the linting process + reduce the token cost.

For the rest, of course, an access to an LLM is required. The goal would be to use an abstraction that allows people to configure the LLM provider of their choice (OpenAI, Gemini, OpenRouter, Ollama, etc.)

Like in ESLint, every problems should have a level, rules should have configuration, etc.
Unlike ESLint, every issue surfaces should be backed by a reputable source of informations with a description + a link.

Like ESLint, when a static Auto-fix is possible, make it possible. We won't auto-fix using LLM this time.

Finally, that would be awesome if we could integrate SLint into editors like VSCode for inline visualization of issues.

Of course, a plugin system must exist to enable people to create plugins easily, like in eslint.

Finally, a beautiful documentation website must be created (another app in "apps") that will allow other to use and contribute to the software.

For maximal speed, this program must be created in Rust.
Target for test coverage should be maximum.

Principles:
- Fast: As fast as possible with static analysis when possible and optional LLM analysis
- Extensible: Every one should be able to contribute easily with their own set of rules
- Configurable: Tailor the rules to what you really need
- Re-use: Whenever possible, the code should use mature libraries and standards to rely on tested, already written code.
- Tested: Every feature, every rules, should be thoroughly tested to ensure non regression over time. Stability is key.