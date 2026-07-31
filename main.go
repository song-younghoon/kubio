package main

import (
	"encoding/json"
	"log"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
)

type config struct {
	Listen string `json:"listen"`
	Proxy  struct {
		Target string `json:"target"`
	} `json:"proxy"`
}

func main() {
	data, err := os.ReadFile("config.json")
	if err != nil {
		log.Fatal(err)
	}

	var cfg config
	if err := json.Unmarshal(data, &cfg); err != nil {
		log.Fatal(err)
	}
	if cfg.Listen == "" || cfg.Proxy.Target == "" {
		log.Fatal("config.json must set listen and proxy.target")
	}

	target, err := url.Parse(cfg.Proxy.Target)
	if err != nil {
		log.Fatal(err)
	}
	if target.Scheme == "" || target.Host == "" {
		log.Fatal("proxy.target must be an absolute URL")
	}

	log.Fatal(http.ListenAndServe(cfg.Listen, httputil.NewSingleHostReverseProxy(target)))
}
