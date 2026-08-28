use std::future::Future;
use std::pin::Pin;
use std::task::{Context, RawWaker, RawWakerVTable, Waker};

use hexcell_core::embeddings::{
    ErrorDeIntegracion, LoteDeEmbeddings, PeticionDeEmbeddings, ProveedorDeEmbeddings,
    RespuestaDeEmbeddings, VectorDeEmbedding,
};

fn bloqueante<F: Future>(mut futuro: F) -> F::Output {
    fn raw_waker() -> RawWaker {
        fn noop(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            raw_waker()
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    let waker = unsafe { Waker::from_raw(raw_waker()) };
    let mut cx = Context::from_waker(&waker);
    let mut futuro = unsafe { Pin::new_unchecked(&mut futuro) };
    match futuro.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(res) => res,
        std::task::Poll::Pending => panic!("el futuro no estuvo listo inmediatamente"),
    }
}

struct ProveedorDePrueba;

impl ProveedorDeEmbeddings for ProveedorDePrueba {
    type Error = std::io::Error;

    async fn incrustar_lote(
        &self,
        peticion: PeticionDeEmbeddings,
    ) -> Result<RespuestaDeEmbeddings, Self::Error> {
        let vectores = peticion
            .textos
            .into_iter()
            .map(|t| Some(VectorDeEmbedding::nuevo(vec![t.len() as f32])))
            .collect();
        Ok(RespuestaDeEmbeddings {
            vectores,
            unidades_consumidas: 10,
        })
    }
}

fn ejecutar_proveedor_generico<P: ProveedorDeEmbeddings>(
    proveedor: &P,
    peticion: PeticionDeEmbeddings,
) -> Result<RespuestaDeEmbeddings, P::Error> {
    bloqueante(proveedor.incrustar_lote(peticion))
}

#[test]
fn puerto_de_embeddings_se_consume_genericamente() {
    let proveedor = ProveedorDePrueba;
    let peticion = PeticionDeEmbeddings {
        textos: vec!["uno".to_string(), "dos".to_string()],
    };

    let respuesta = ejecutar_proveedor_generico(&proveedor, peticion)
        .expect("la ejecución genérica del puerto debe ser exitosa");
    assert_eq!(respuesta.vectores.len(), 2);
    assert_eq!(respuesta.unidades_consumidas, 10);
}

#[test]
fn vector_de_embedding_conversion_a_bytes_le_y_reconstruccion() {
    let valores_originales = vec![1.0f32, -2.5f32, 3.14159f32, 0.0f32];
    let vector = VectorDeEmbedding::nuevo(valores_originales.clone());

    assert_eq!(vector.dimension(), 4);
    assert_eq!(vector.valores(), &valores_originales);

    let bytes = vector.a_bytes_le();
    assert_eq!(
        bytes.len(),
        16,
        "4 componentes f32 deben ocupar exactamente 16 bytes"
    );

    let mut bytes_esperados = Vec::new();
    for val in &valores_originales {
        bytes_esperados.extend_from_slice(&val.to_le_bytes());
    }
    assert_eq!(bytes, bytes_esperados);

    let vector_reconstruido =
        VectorDeEmbedding::desde_bytes_le(&bytes).expect("debe deserializarse correctamente");
    assert_eq!(vector, vector_reconstruido);
}

#[test]
fn vector_de_embedding_rechaza_bytes_con_longitud_no_multiplo_de_cuatro() {
    let bytes_invalidos = vec![0u8; 15];
    assert!(VectorDeEmbedding::desde_bytes_le(&bytes_invalidos).is_none());

    let bytes_validos = vec![0u8; 16];
    assert!(VectorDeEmbedding::desde_bytes_le(&bytes_validos).is_some());
}

#[test]
fn respuesta_de_embeddings_mantiene_correspondencia_posicional() {
    let textos = vec!["alfa".to_string(), "beta".to_string(), "gamma".to_string()];
    let vectores = vec![
        Some(VectorDeEmbedding::nuevo(vec![0.1])),
        None,
        Some(VectorDeEmbedding::nuevo(vec![0.3])),
    ];

    let respuesta = RespuestaDeEmbeddings {
        vectores,
        unidades_consumidas: 5,
    };

    assert_eq!(respuesta.vectores.len(), textos.len());
    assert!(respuesta.vectores[0].is_some());
    assert!(respuesta.vectores[1].is_none());
    assert!(respuesta.vectores[2].is_some());
}

#[test]
fn lote_de_embeddings_gestion_de_resuncion_y_completitud() {
    let textos = vec![
        "fragmento 0".to_string(),
        "fragmento 1".to_string(),
        "fragmento 2".to_string(),
    ];
    let mut lote = LoteDeEmbeddings::nuevo(textos);

    assert_eq!(lote.pendientes(), 3);
    assert!(!lote.esta_completo());
    assert!(lote.clone().completo().is_none());

    let (peticion_1, indices_1) = lote
        .peticion_pendiente()
        .expect("debe haber fragmentos pendientes");
    assert_eq!(indices_1, vec![0, 1, 2]);
    assert_eq!(peticion_1.textos.len(), 3);

    let respuesta_1 = RespuestaDeEmbeddings {
        vectores: vec![
            Some(VectorDeEmbedding::nuevo(vec![0.0])),
            None,
            Some(VectorDeEmbedding::nuevo(vec![2.0])),
        ],
        unidades_consumidas: 2,
    };

    lote.integrar(&indices_1, respuesta_1)
        .expect("la integración debe ser exitosa");

    assert_eq!(lote.pendientes(), 1);
    assert!(!lote.esta_completo());
    assert!(lote.clone().completo().is_none());

    let (peticion_2, indices_2) = lote
        .peticion_pendiente()
        .expect("debe haber 1 fragmento pendiente");
    assert_eq!(indices_2, vec![1]);
    assert_eq!(peticion_2.textos, vec!["fragmento 1".to_string()]);

    let respuesta_2 = RespuestaDeEmbeddings {
        vectores: vec![Some(VectorDeEmbedding::nuevo(vec![1.0]))],
        unidades_consumidas: 1,
    };

    lote.integrar(&indices_2, respuesta_2)
        .expect("la segunda integración debe ser exitosa");

    assert_eq!(lote.pendientes(), 0);
    assert!(lote.esta_completo());

    let vectores_finales = lote.completo().expect("el lote debe estar completo");
    assert_eq!(vectores_finales.len(), 3);
    assert_eq!(vectores_finales[0].valores(), &[0.0]);
    assert_eq!(vectores_finales[1].valores(), &[1.0]);
    assert_eq!(vectores_finales[2].valores(), &[2.0]);
}

#[test]
fn lote_de_embeddings_integrar_rechaza_longitud_incompatible() {
    let mut lote = LoteDeEmbeddings::nuevo(vec!["a".to_string(), "b".to_string()]);
    let respuesta = RespuestaDeEmbeddings {
        vectores: vec![Some(VectorDeEmbedding::nuevo(vec![1.0]))],
        unidades_consumidas: 1,
    };

    let error = lote
        .integrar(&[0, 1], respuesta)
        .expect_err("debe fallar por discrepancia de longitudes");
    assert_eq!(
        error,
        ErrorDeIntegracion::LongitudIncompatible {
            esperado: 2,
            recibido: 1
        }
    );
}

#[test]
fn lote_de_embeddings_integrar_rechaza_posicion_ya_resuelta() {
    let mut lote = LoteDeEmbeddings::nuevo(vec!["a".to_string()]);
    let respuesta_1 = RespuestaDeEmbeddings {
        vectores: vec![Some(VectorDeEmbedding::nuevo(vec![1.0]))],
        unidades_consumidas: 1,
    };
    lote.integrar(&[0], respuesta_1)
        .expect("primera integración");

    let respuesta_2 = RespuestaDeEmbeddings {
        vectores: vec![Some(VectorDeEmbedding::nuevo(vec![2.0]))],
        unidades_consumidas: 1,
    };
    let error = lote
        .integrar(&[0], respuesta_2)
        .expect_err("debe rechazar sobreescribir un índice resuelto");
    assert_eq!(error, ErrorDeIntegracion::IndiceYaResuelto(0));
}
