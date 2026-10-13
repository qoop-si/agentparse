export default {
  name: "agentparse",
  description: "One parser for Claude Code, Codex, OpenCode and Gemini CLI sessions",

  navLinks: [
    { label: "Docs", href: "#introduction" },
    { label: "Model", href: "#model" },
    { label: "CLI", href: "#cli" },
    { label: "GitHub", href: "https://github.com/qoop-si/agentparse", external: true },
  ],

  sidebar: [
    {
      title: "Getting Started",
      children: [
        { title: "Introduction", slug: "introduction" },
        { title: "Installation", slug: "installation" },
        { title: "Quick Start", slug: "quick-start" },
      ],
    },
    {
      title: "The Model",
      children: [
        { title: "Sessions and Entries", slug: "model" },
        { title: "Tool Calls", slug: "tools" },
        { title: "Shell Commands", slug: "shell" },
        { title: "Usage and Status", slug: "conventions" },
      ],
    },
    {
      title: "Agents",
      children: [
        { title: "Claude Code", slug: "claude" },
        { title: "Codex", slug: "codex" },
        { title: "OpenCode", slug: "opencode" },
        { title: "Gemini CLI", slug: "gemini" },
      ],
    },
    {
      title: "Reference",
      children: [
        { title: "Rust API", slug: "api" },
        { title: "CLI", slug: "cli" },
      ],
    },
  ],

  defaultPage: "introduction",

  footer: {
    left: "MIT License",
    right: "agentparse",
  },
};
