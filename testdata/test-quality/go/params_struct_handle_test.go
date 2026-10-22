// Label: the testing handle carried in a struct literal field into an unresolved verification
// call (`CheckConsumeContract(Params{T: t})`) means the callee asserts. A non-verification name,
// a literal that never reaches a call, and a call without the handle are reported.
package fixtures

import "testing"

func TestLiteralArg(t *testing.T) {
	CheckConsumeContract(Params{T: t, Signal: 1})
}

func TestVariableArg(t *testing.T) {
	params := Params{
		T:      t,
		Signal: 1,
	}

	CheckConsumeContract(params)
}

func TestPointerLiteralArg(t *testing.T) {
	VerifyContract(&Params{T: t})
}

func TestNonVerifyingName(t *testing.T) {
	newServer(Params{T: t})
}

func TestCarrierNeverPassed(t *testing.T) {
	params := Params{T: t}
	_ = params
	prepare()
}

func TestVerifyWithoutHandle(t *testing.T) {
	ValidateContract(Params{Signal: 1})
}
