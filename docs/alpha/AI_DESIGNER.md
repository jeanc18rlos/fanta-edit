# AI designer controls

The native Fanta designer reads `fanta.md` from the active project before each
model round. Use it to describe the audience, visual direction, typography,
spacing, asset preferences, design-system conventions, and acceptance criteria.
Nested project rules refine workspace rules. User instructions and mode/tool
permissions take precedence. Keep the combined specifications below 64 KiB.

```markdown
# Product design rules

Audience: independent studios managing client projects.
Direction: quiet, spacious, editorial; one saturated blue accent.
Typography: reuse the project's text styles, with a clear heading/body hierarchy.
Layout: use nested auto-layout frames; center button icon/label rows; align card
content to consistent padding. Use semantic spacing variables.
Assets: offer a cohesive SVG icon set; ask before introducing generated imagery.
Systems: reuse existing variables and components, with light/dark modes where needed.
Acceptance: check wrapping, alignment, contrast, clipping, and screenshots at
desktop and narrow widths before finishing.
```

**Edit Visual** applies undoable canvas operations directly. **Write** and
**Full Access** can edit saved FNX source; **Ultra** coordinates specialists for
larger tasks. **Plan** and **Review** preserve the scene. `design_system` exposes
the supported variable, collection, mode, binding, and component schema; agents
should inspect it before creating foundations.

The composer displays Thinking and an effort slider using the selected model's
reported capabilities. Unsupported models show an unavailable state. Effort
selection changes the actual request and is persisted for matching configured
models. It does not expose private reasoning or add unsupported provider options.

Generated fenced code folds while streaming and stays folded after completion.
Manual expansion remains stable as new text arrives. A single source writer opens
the page's Code workspace and follows its shared buffer. **Pause follow** stops
scrolling. Returning to Canvas is respected; another chunk does not switch it
back. Multiple agents retain their canvas activity and individual cursors.

The designer offers relevant images or SVG icons. `prepare_design_asset` opens
the existing generation workspace with a tailored prompt, without submitting a
generation. A preferred model is selected only when a compatible entry exists in
the account's catalog. Review and submit there, then use the existing canvas
placement flow. OpenAI image availability follows the server catalog and account
access; this client does not manufacture model availability.

## Research informing the guidance

The prompts use original, concise guidance informed by these primary sources:

- [Anthropic frontend-design skill](https://github.com/anthropics/skills/blob/main/skills/frontend-design/SKILL.md): deliberate direction, typography, composition, and avoiding repetitive defaults.
- [OpenAI design-system workflow](https://github.com/openai/plugins/blob/main/plugins/figma/skills/figma-generate-library/SKILL.md): foundations before components, reusable variables, and mode-aware systems.
- [OpenAI frontend design guidance](https://developers.openai.com/blog/designing-delightful-frontends-with-gpt-5-4): explicit briefs, purposeful visual hierarchy, and visual verification.
- [GOV.UK spacing](https://design-system.service.gov.uk/styles/spacing/): consistent spacing rhythm.
- [Design Tokens Community Group format](https://www.designtokens.org/TR/2025.10/format/): typed tokens and aliases.
- [OpenAI image generation](https://developers.openai.com/api/docs/guides/image-generation) and [reasoning controls](https://developers.openai.com/api/docs/guides/reasoning): provider-specific image workflows and supported reasoning effort.

These references inform implementation; they are not installed dependencies or
imported instructions that override a user's project rules.
