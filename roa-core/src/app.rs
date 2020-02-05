mod executor;
mod tcp;

use crate::{
    default_status_handler, last, Context, DynTargetHandler, Middleware, Model, Next, Request,
    Response, Status, TargetHandler,
};
use executor::Executor;
use http::{Request as HttpRequest, Response as HttpResponse};
use hyper::service::Service;
use hyper::Body as HyperBody;
use hyper::Server;
use std::future::Future;
use std::net::{SocketAddr, ToSocketAddrs};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
pub use tcp::{AddrIncoming, AddrStream};

pub struct App<M: Model> {
    middleware: Middleware<M>,
    status_handler: Arc<DynTargetHandler<M, Status>>,
    pub(crate) model: Arc<M>,
}

pub struct HttpService<M: Model> {
    app: App<M>,
    stream: AddrStream,
}

impl<M: Model> App<M> {
    pub fn new(model: M) -> Self {
        Self {
            middleware: Middleware::new(),
            status_handler: Arc::from(Box::new(default_status_handler).dynamic()),
            model: Arc::new(model),
        }
    }

    pub fn gate<F>(
        &mut self,
        middleware: impl 'static + Sync + Send + Fn(Context<M>, Next) -> F,
    ) -> &mut Self
    where
        F: 'static + Future<Output = Result<(), Status>> + Send,
    {
        self.middleware.join(middleware);
        self
    }

    pub fn handle_status<F>(
        &mut self,
        handler: impl 'static + Sync + Send + Fn(Context<M>, Status) -> F,
    ) -> &mut Self
    where
        F: 'static + Future<Output = Result<(), Status>> + Send,
    {
        self.status_handler = Arc::from(Box::new(handler).dynamic());
        self
    }

    fn listen_on(
        &self,
        addr: impl ToSocketAddrs,
    ) -> std::io::Result<(SocketAddr, hyper::Server<AddrIncoming, App<M>, Executor>)> {
        let incoming = AddrIncoming::bind(addr)?;
        let local_addr = incoming.local_addr();
        let server = Server::builder(incoming)
            .executor(Executor::new())
            .serve(self.clone());
        Ok((local_addr, server))
    }

    pub fn listen(
        &self,
        addr: impl ToSocketAddrs,
        callback: impl Fn(SocketAddr),
    ) -> std::io::Result<hyper::Server<AddrIncoming, App<M>, Executor>> {
        let (addr, server) = self.listen_on(addr)?;
        callback(addr);
        Ok(server)
    }

    pub fn run(
        &self,
    ) -> std::io::Result<(SocketAddr, hyper::Server<AddrIncoming, App<M>, Executor>)> {
        self.listen_on("0.0.0.0:0")
    }

    pub fn run_local(
        &self,
    ) -> std::io::Result<(SocketAddr, hyper::Server<AddrIncoming, App<M>, Executor>)> {
        self.listen_on("127.0.0.1:0")
    }
}

macro_rules! impl_poll_ready {
    () => {
        fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    };
}

type AppFuture<M> =
    Pin<Box<dyn 'static + Future<Output = Result<HttpService<M>, std::io::Error>> + Send>>;

impl<M: Model> Service<&AddrStream> for App<M> {
    type Response = HttpService<M>;
    type Error = std::io::Error;
    type Future = AppFuture<M>;
    impl_poll_ready!();
    fn call(&mut self, stream: &AddrStream) -> Self::Future {
        let app = self.clone();
        let stream = stream.clone();
        Box::pin(async move { Ok(HttpService::new(app, stream)) })
    }
}

type HttpFuture =
    Pin<Box<dyn 'static + Future<Output = Result<HttpResponse<HyperBody>, Status>> + Send>>;

impl<M: Model> Service<HttpRequest<HyperBody>> for HttpService<M> {
    type Response = HttpResponse<HyperBody>;
    type Error = Status;
    type Future = HttpFuture;
    impl_poll_ready!();
    fn call(&mut self, req: HttpRequest<HyperBody>) -> Self::Future {
        let service = self.clone();
        Box::pin(async move { Ok(service.serve(req.into()).await?.into()) })
    }
}

impl<M: Model> HttpService<M> {
    pub fn new(app: App<M>, stream: AddrStream) -> Self {
        Self { app, stream }
    }

    pub async fn serve(&self, req: Request) -> Result<Response, Status> {
        let context = Context::new(req, self.app.clone(), self.stream.clone());
        let app = self.app.clone();
        if let Err(status) = (app.middleware.handler())(context.clone(), Box::new(last)).await {
            (app.status_handler)(context.clone(), status).await?;
        }
        let mut response = context.resp_mut().await;
        Ok(std::mem::take(&mut *response))
    }
}

impl<M: Model> Clone for App<M> {
    fn clone(&self) -> Self {
        Self {
            middleware: self.middleware.clone(),
            status_handler: self.status_handler.clone(),
            model: self.model.clone(),
        }
    }
}

impl<M: Model> Clone for HttpService<M> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            stream: self.stream.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::App;
    use async_std::task::spawn;
    use http::StatusCode;
    use std::time::Instant;

    #[tokio::test]
    async fn gate_simple() -> Result<(), Box<dyn std::error::Error>> {
        let (addr, server) = App::new(())
            .gate(|_ctx, next| async move {
                let inbound = Instant::now();
                next().await?;
                println!("time elapsed: {} ms", inbound.elapsed().as_millis());
                Ok(())
            })
            .run_local()?;
        spawn(server);
        let resp = reqwest::get(&format!("http://{}", addr)).await?;
        assert_eq!(StatusCode::OK, resp.status());
        Ok(())
    }
}
