// Choices for the Claude session, shared by the PR page (content.js) and the settings page.
// Values are Claude Code's own: model aliases and effort levels; "default" leaves the setting alone.
const CLAUDE_MODELS = [
  ["default", "Default"],
  ["fable", "Fable"],
  ["opus", "Opus"],
  ["opus[1m]", "Opus (1M context)"],
  ["sonnet", "Sonnet"],
  ["haiku", "Haiku"],
];
const CLAUDE_EFFORTS = [
  ["default", "Default"],
  ["low", "Low"],
  ["medium", "Medium"],
  ["high", "High"],
  ["xhigh", "Extra high"],
  ["max", "Max"],
];

function fillSelect(select, options, value) {
  select.replaceChildren(...options.map(([v, label]) => new Option(label, v, false, v === value)));
}
