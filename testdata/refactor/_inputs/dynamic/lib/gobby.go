package lib

import "encoding/gob"

// Thing is registered with gob under its package-qualified name.
type Thing struct{ N int }

// RegisterAll registers wire types.
func RegisterAll() { gob.Register(Thing{}) }
