create-self-signed-ssl-certificate:
  openssl req -newkey rsa:2048 -nodes -keyout ./cert/key.pem -x509 -days 365 -out ./cert/certificate.pem

test-delete:
  curl -i --insecure -X DELETE https://localhost:3443/api/posts/test -H 'Authorization: Bearer TODO'

test-upload:
  touch test.md
  echo -e "# Title\nthis is a test 🍰!" > test.md
  curl -i --insecure --form file='@test.md' https://localhost:3443/api/post -H 'Authorization: Bearer TODO'

