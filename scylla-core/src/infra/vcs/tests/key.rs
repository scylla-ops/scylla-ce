//! A test RSA key pair: the private key signs the JWT of a test App, and the public key checks
//! it. It is not a key of a real App.

/// The private key, in PKCS #1 PEM, as GitHub gives the key of an App.
pub(super) const PRIVATE_KEY: &str = "\
-----BEGIN RSA PRIVATE KEY-----\n\
MIIEoQIBAAKCAQEAygYfCQc3Voltt8jxfPnC0PAswe2tQtl3TwKWvstpLQlAyuya\n\
EmquWJ+0XkjDLQTmAsSyLx8a0/nfGMThVT6eir7CP+y57ieprPpb0aRTcXRwcHVp\n\
xxCoBBsQ5R/6MzAAQvkjjrR72MVSPLYpdUcZTkfB0spzzty2yJWmRQ+NUsydFTU8\n\
dVPL6/1RcObzsM5fpesJy8QfGxWVuDs9PcNxQskUtKpy9L0lcsBA/uEsipArez4J\n\
4zr1Sf+WuYhRHJmbM52yMlyqbi+WPFSHbFdeyvHUDHiMMkZ6aJjctlR62t6Wzv2h\n\
1YkamXKIS8u289e6pJfKruS9J3vbhuI3Il6zRQIDAQABAoIBABKVv5MoC8o1OFe4\n\
76ZMn8Ry49svT0ogEV6+YTnKIrHW8RP58i6Ri9pxTrmBvhrNIY/Nooq9NM2dZg20\n\
Rvp9mT4wUFRrJauoEG7uVTYTBO4iBRoDYyjB+oLuLn61k2HUH6IJo2DEb8RAWtFk\n\
/KliUgymg0kuALteCJ6pTbN6u8iLCmyFRIlcOFrHSCt76wc5oKQwLq2y+vYeo0rR\n\
dGcjeBgPJtVj9yBZINQNd9CClflRFtteSPkxeQXzrzprl/n8gVlRToQx1Dw3Qr1O\n\
gD8nO8JzfZIMybTLDFHeRKZioPxy00+8hgRoctEKnoT7d/PsL+6A2F1w3/XZy0VJ\n\
HhddAv0CgYEA5NOcd2vELS63erfiqbieknBbgg1wODxnep0CXpx0P04CxdPxgYbH\n\
KVFnl/lvyeXQyNmysQNaumA9/ll8VHTObga5QOhzwv4E/gzwnlIEj22KYkWZ2g/E\n\
gyhrr2jP0pmWb/+71NjvczOeyuyu9Wd2aJ+SHUPuX8mAHeRce7ZBsdMCgYEA4gOz\n\
ebn/pTxq88a6YmD0JiGcfJTFwV9FNWm+fITdmdY8BAmp7qf+rpaG/cZ0omHeP1N+\n\
gmGpXFrNDVocGv2hVnSU9jbUdjvq8llf2m0EmYmCyedhUUxM13466ZduhOGfzVyp\n\
Hq94cIRLwSAD4FvXPfLQ0YpUyCtWWSrvO5fNP4cCgYAjKBPxESCZ5ISSGTB87P2H\n\
+DVG/POR4E3xu6/Cg30kwMKSTGbdhKySv4wQAGXZejKiuTfIFIDbA7BipLMCctPI\n\
ecX6n/wxKD/yHGDXojhLJQRO5uylYSjLEenBkHPlom1Hld3w7MeVRogOG2EiGRxL\n\
JMEZN/emxdmmPhCrfSwIuwKBgQChJE5kLY7tuoPuxvPAjKvGPBScZl4QnuJzUK91\n\
jC2LL0me/1zfQXu98qjQaxxhYjHZKKh8QTFFHaAxMtvzCn5ra7eBTFpy6gl7POmw\n\
jCXJaleLqKKkGih3RBKw1T0BFPSgsL4LmMPgRRLV4QOihRiTnHnxzBTimrN347qp\n\
3HOEJQJ/P0fj7At+jBKUawo7IRKu7vffjhTVddnHNSXe95cNQJ4DfWRfEllKcI9Y\n\
jiRILkXzX1JZfdnFsSUw5pkDFKQRrWeIZQgmP2bNzKO424+0Fdd7vA6RJ1E/Q/lE\n\
10p7vH1k+oeEUm/wvwtxnkyHO50rQnAb7p+zfDGrMA8NjkXiRw==\n\
-----END RSA PRIVATE KEY-----\n";

/// The public key of `PRIVATE_KEY`, in PEM.
pub(super) const PUBLIC_KEY: &str = "\
-----BEGIN PUBLIC KEY-----\n\
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAygYfCQc3Voltt8jxfPnC\n\
0PAswe2tQtl3TwKWvstpLQlAyuyaEmquWJ+0XkjDLQTmAsSyLx8a0/nfGMThVT6e\n\
ir7CP+y57ieprPpb0aRTcXRwcHVpxxCoBBsQ5R/6MzAAQvkjjrR72MVSPLYpdUcZ\n\
TkfB0spzzty2yJWmRQ+NUsydFTU8dVPL6/1RcObzsM5fpesJy8QfGxWVuDs9PcNx\n\
QskUtKpy9L0lcsBA/uEsipArez4J4zr1Sf+WuYhRHJmbM52yMlyqbi+WPFSHbFde\n\
yvHUDHiMMkZ6aJjctlR62t6Wzv2h1YkamXKIS8u289e6pJfKruS9J3vbhuI3Il6z\n\
RQIDAQAB\n\
-----END PUBLIC KEY-----\n";
