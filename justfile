create-self-signed-ssl-certificate:
  openssl req -newkey rsa:2048 -nodes -keyout ./key.pem -x509 -days 365 -out ./certificate.pem

test-upload:
  touch test.md
  echo -e "# Title\nthis is a test 🍰!" > test.md
  curl -i --insecure --form file='@test.md' https://localhost:3443/post

