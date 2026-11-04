package lib

var registry = map[string]int{}

func init() { registry["x"] = 1 }

// Registered reports the registry size.
func Registered() int { return len(registry) }
