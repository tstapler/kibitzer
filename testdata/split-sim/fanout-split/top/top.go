package top

import (
	"ex.com/ss/app1"
	"ex.com/ss/app2"
)

func Run() int { return app1.Run() + app2.Run() }
