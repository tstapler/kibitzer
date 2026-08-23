// Package infra is the dogfood fixture's lowest layer: no internal imports. It
// deliberately breaks naming-rules (see ../../.claude/inspect.json): infra's naming
// rule requires struct names to end in "Repository" or "Client", but OrderStore
// doesn't match.
package infra

// OrderStore is the deliberate naming-rules violation — it should be named
// OrderRepository or OrderClient to satisfy infra's naming rule.
type OrderStore struct{}

// Save is a placeholder persistence method so domain.Validate has something real
// to call across the (deliberately denied) domain -> infra edge.
func (s OrderStore) Save(id string) {
	_ = id
}
