// Package handlers is the dogfood fixture's top layer: it may depend on domain,
// exercising kibitzer's component-deps/content-rules/naming-rules checkers
// end-to-end (see testdata/dogfood-architecture and ../../.claude/inspect.json).
package handlers

import "dogfood.example/app/domain"

// CreateOrder validates an order via the domain layer. This import is allowed —
// handlers -> domain is not denied by any dependency rule.
func CreateOrder(o domain.Order) error {
	return domain.Validate(o)
}
