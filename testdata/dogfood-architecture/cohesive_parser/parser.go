// Package cohesive_parser is the negative fixture: every type and function shares
// ParseContext, so the checker must not propose a split.
package cohesive_parser

type ParseContext struct{ input string; pos int }

type Lexer struct{ ctx *ParseContext }

type Grammar struct{ ctx *ParseContext }

func (l *Lexer) Advance() { l.ctx.pos++ }

func (g *Grammar) Rule() int { return g.ctx.pos }

func Scan(ctx *ParseContext, l *Lexer) { l.Advance(); ctx.pos++ }

func Parse(ctx *ParseContext, g *Grammar) int { return g.Rule() + ctx.pos }
