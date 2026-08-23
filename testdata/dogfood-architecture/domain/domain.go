// Package domain is the dogfood fixture's business-rules layer: it should depend on
// nothing but itself. It deliberately breaks that rule twice, to exercise
// component-deps and content-rules (see ../../.claude/inspect.json):
//
//   - importing dogfood.example/app/infra is a component-deps violation — domain's
//     dependency_rule only allows depending on domain.
//   - the Validate function is a content-rules violation — domain's content_rule only
//     allows "struct" declarations, not "function".
package domain

import "dogfood.example/app/infra"

// Order is domain's one allowed declaration kind: a plain struct.
type Order struct {
	ID string
}

// Validate is the deliberate content-rules violation: domain's content rule allows
// only "struct", not "function". It also reaches into infra (the deliberate
// component-deps violation) instead of depending only on domain.
func Validate(o Order) error {
	infra.OrderStore{}.Save(o.ID)
	return nil
}
