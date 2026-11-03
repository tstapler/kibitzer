package lib

import "strings"

// Util upper-cases.
func Util() string { return strings.ToUpper("x") }

// Widget doubles.
type Widget struct{ N int }

func (w Widget) Double() int { return w.N * 2 }
