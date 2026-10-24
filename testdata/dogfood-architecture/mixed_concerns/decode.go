// Package mixed_concerns is the positive go-package-focus fixture: decoder types and
// storage types never reference each other, so the checker should propose a split.
package mixed_concerns

type Decoder struct{ pos int }

type Token struct{ kind string }

func (d *Decoder) Next() int { d.pos++; return d.pos }

func (t *Token) Kind() string { return t.kind }

func ReadToken(d *Decoder, t *Token) string { d.Next(); return t.Kind() }
