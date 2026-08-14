/** One catalogue entry from `slint rules --json` / `src/data/rules.json`. */
export type Rule = {
  name: string
  summary: string
  rationale: string
  advice: string
  default_severity: string
  fixable: boolean
  needs_model: boolean
  reference_title: string
  reference_url: string
}

export type RuleArea =
  | 'name'
  | 'description'
  | 'frontmatter'
  | 'body'
  | 'bundle'
  | 'project'
  | 'llm'
