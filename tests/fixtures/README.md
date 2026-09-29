These certificates and keys are synthetic, public test fixtures used only by
the offline loopback TLS server. They contain no provider or device data.

`localhost-cert-reissued.pem` has the same public key as `localhost-cert.pem`
because it was signed with `localhost-key.pem`. Its serial number and validity
period differ. `other-cert.pem` uses `other-key.pem`, so its public key differs.
All three certificates name `localhost` and `127.0.0.1` for local tests.

The additional fixtures were generated locally, without network access, using
OpenSSL 3.0.20:

```sh
openssl req -new -x509 -key tests/fixtures/localhost-key.pem -sha256 \
  -days 1825 -set_serial 0x2026092601 -subj /CN=localhost \
  -addext subjectAltName=DNS:localhost,IP:127.0.0.1 \
  -addext extendedKeyUsage=serverAuth \
  -out tests/fixtures/localhost-cert-reissued.pem
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
  -out tests/fixtures/other-key.pem
openssl req -new -x509 -key tests/fixtures/other-key.pem -sha256 \
  -days 1825 -set_serial 0x2026092602 -subj /CN=localhost \
  -addext subjectAltName=DNS:localhost,IP:127.0.0.1 \
  -addext extendedKeyUsage=serverAuth \
  -out tests/fixtures/other-cert.pem
```

To check the key relationship without displaying private key material:

```sh
for cert in tests/fixtures/localhost-cert.pem \
            tests/fixtures/localhost-cert-reissued.pem \
            tests/fixtures/other-cert.pem; do
  openssl x509 -in "$cert" -pubkey -noout \
    | openssl pkey -pubin -outform der \
    | openssl dgst -sha256 -hex
done
```

The two localhost certificates produce the same SPKI SHA-256 digest;
`other-cert.pem` produces a different digest. Regenerating a certificate
changes its validity dates and certificate hash, so update any expected test
hashes if these fixtures are refreshed.
