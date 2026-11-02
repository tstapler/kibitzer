package lib

import "sync"

type Store struct {
	mu sync.Mutex
	n  int
}

func NewStore() *Store { return &Store{} }
