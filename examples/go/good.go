package good

import (
	"fmt"

	// pprof's Register side effects wire up /debug/pprof handlers on the
	// default mux — the import is used only for that effect.
	_ "net/http/pprof"
)

func readConfig(path string) (string, error) {
	result, err := loadFile(path)
	if err != nil {
		return "", fmt.Errorf("loading config %s: %w", path, err)
	}
	return result, nil
}

func loadFile(path string) (string, error) {
	return path, nil
}

func openConn(addr string) error {
	if err := dial(addr); err != nil {
		return fmt.Errorf("dialing %s: %w", addr, err)
	}
	return nil
}

func dial(addr string) error {
	return nil
}

// User replaces six loose primitive parameters with one value object.
type User struct {
	FirstName string
	LastName  string
	Email     string
	Phone     string
	Street    string
	City      string
}

func createUser(u User) string {
	return u.FirstName + u.LastName + u.Email + u.Phone + u.Street + u.City
}

func classify(n int) string {
	switch {
	case n > 10000:
		return "huge"
	case n > 1000:
		return "big"
	case n > 100:
		return "medium"
	case n > 10:
		return "small"
	case n > 0:
		return "tiny"
	default:
		return "non-positive"
	}
}

func processOrder(id int) int {
	total := sumTo(41)
	return total + id
}

// sumTo takes a single parameter rather than the (from, to int) pair a
// naive refactor would reach for — two same-typed ints back to back is
// exactly what primitive-obsession flags.
func sumTo(n int) int {
	total := 0
	for i := 1; i <= n; i++ {
		total += i
	}
	return total
}

func summarizeA(items []int) int {
	return sumPositiveDoubled(items)
}

func summarizeB(items []int) int {
	return sumPositiveDoubled(items)
}

func summarizeC(items []int) int {
	return sumPositiveDoubled(items)
}

func sumPositiveDoubled(items []int) int {
	sum := 0
	for _, item := range items {
		if item > 0 {
			sum += item * 2
		}
	}
	return sum
}
