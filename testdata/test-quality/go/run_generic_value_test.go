// Label: t.Run's body argument in every shape that names or builds a func(*testing.T) --
// bare generic instantiation (`f[T]`, `f[T1, T2]`), a call of one (`f[T]()`), parenthesized,
// or a method value on those. A same-file target is followed (a returned closure included);
// a target outside the file is unknowable (quiet); an empty same-file target is still flagged.
package fixtures

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func assertingGeneric[T any](t *testing.T) {
	var zero T
	assert.Equal(t, zero, zero)
}

func assertingPair[A, B any](t *testing.T) {
	assert.Equal(t, 1, 1)
}

func assertingGenericClosure[T any]() func(*testing.T) {
	return func(t *testing.T) {
		assert.Equal(t, 1, 1)
	}
}

func emptyGeneric[T any](t *testing.T) {}

func emptyGenericClosure[T any]() func(*testing.T) {
	return func(t *testing.T) {}
}

func TestGenericValueAsserts(t *testing.T) {
	t.Run("Int64", assertingGeneric[int64])
	t.Run("Float64", assertingGeneric[float64])
}

func TestGenericValueTwoTypeArgs(t *testing.T) {
	t.Run("pair", assertingPair[int, string])
}

func TestGenericCallAsserts(t *testing.T) {
	t.Run("call", assertingGenericClosure[int]())
}

func TestGenericValueParenthesized(t *testing.T) {
	t.Run("paren", (assertingGeneric[int]))
}

func TestGenericValueOutsideFile(t *testing.T) {
	t.Run("Int64", otherpkg.Helper[int64])
	t.Run("Float64", unseenGeneric[float64])
}

func TestGenericCallOutsideFile(t *testing.T) {
	t.Run("call", otherpkg.Closure[int, string]())
	t.Run("call2", unseenClosure[int]())
}

type holder struct{}

func (holder) body(t *testing.T) { assert.True(t, true) }

func TestGenericMethodValueOnCall(t *testing.T) {
	t.Run("method", newHolder[int]().body)
}

// Flagged: the same-file generic target asserts nothing.
func TestGenericValueEmpty(t *testing.T) {
	t.Run("Int64", emptyGeneric[int])
}

// Flagged: the same-file generic call returns an empty closure.
func TestGenericCallEmptyClosure(t *testing.T) {
	t.Run("call", emptyGenericClosure[int]())
}

// Flagged: an inline empty closure.
func TestInlineEmptyClosure(t *testing.T) {
	t.Run("inline", func(t *testing.T) {})
}

var bodies = []func(*testing.T){assertingGeneric[int]}

func TestRangeOverSameFileFuncValues(t *testing.T) {
	for _, f := range bodies {
		t.Run("body", f)
	}
}

func TestRunnerLoopOverFuncValues(t *testing.T) {
	for _, f := range bodies {
		runner.Run("body", f)
	}
}

func runWith(t *testing.T, body func(*testing.T)) { body(t) }

func TestGenericValueHandedToRunner(t *testing.T) {
	runWith(t, assertingGeneric[int])
}
