use serde::{Serialize, Deserialize};
use super::master::{self, Master, Topic};
use super::slave::Slave;
use super::error::{ErrorKind, Result};
use super::super::rosxmlrpc::Response;
use super::naming::{self, Resolver};
use super::resolve;
use tcpros::{Client, Message, PublisherStream, ServicePair, ServiceResult};
use xml_rpc;
use std::time::Duration;

pub struct Ros {
    master: Master,
    slave: Slave,
    hostname: String,
    resolver: Resolver,
    name: String,
}

impl Ros {
    pub fn new(name: &str) -> Result<Ros> {
        let namespace = resolve::namespace();
        let master_uri = resolve::master();
        let hostname = resolve::hostname();
        let name = resolve::name(name);
        let mut ros = Ros::new_raw(&master_uri, &hostname, &namespace, &name)?;
        for (src, dest) in resolve::mappings() {
            ros.map(&src, &dest)?;
        }
        Ok(ros)
    }

    fn new_raw(master_uri: &str, hostname: &str, namespace: &str, name: &str) -> Result<Ros> {
        let namespace = namespace.trim_right_matches("/");

        if name.contains("/") {
            bail!(ErrorKind::Naming(
                naming::error::ErrorKind::IllegalCharacter(name.into()),
            ));
        }

        let name = format!("{}/{}", namespace, name);
        let resolver = Resolver::new(&name)?;

        let slave = Slave::new(&master_uri, &hostname, 0, &name)?;
        let master = Master::new(&master_uri, &name, &slave.uri());
        Ok(Ros {
            master: master,
            slave: slave,
            hostname: String::from(hostname),
            resolver: resolver,
            name: name,
        })
    }

    fn map(&mut self, source: &str, destination: &str) -> Result<()> {
        self.resolver.map(source, destination).map_err(|v| v.into())
    }

    pub fn uri(&self) -> &str {
        return self.slave.uri();
    }

    pub fn name(&self) -> &str {
        return &self.name;
    }

    pub fn hostname(&self) -> &str {
        return &self.hostname;
    }

    pub fn param<'a, 'b>(&'a self, name: &'b str) -> Option<Parameter<'a>> {
        self.resolver.translate(name).ok().map(|v| {
            Parameter {
                master: &self.master,
                name: v,
            }
        })
    }

    pub fn parameters(&self) -> Response<Vec<String>> {
        self.master.get_param_names()
    }

    pub fn state(&self) -> Response<master::SystemState> {
        self.master.get_system_state().map(Into::into)
    }

    pub fn topics(&self) -> Response<Vec<Topic>> {
        self.master.get_topic_types().map(|v| {
            v.into_iter().map(Into::into).collect()
        })
    }

    pub fn client<T: ServicePair>(&self, service: &str) -> Result<Client<T>> {
        let name = self.resolver.translate(service)?;
        let uri = self.master.lookup_service(&name)?;
        Ok(Client::new(&self.name, &uri, &name))
    }

    pub fn wait_for_service(&self, service: &str, timeout: Option<Duration>) -> Result<()> {
        use rosxmlrpc::ResponseError;
        use std::thread::sleep;

        let name = self.resolver.translate(service)?;
        let now = ::std::time::Instant::now();
        loop {
            let e = match self.master.lookup_service(&name) {
                Ok(_) => return Ok(()),
                Err(e) => e,
            };
            match e {
                ResponseError::Client(ref m) if m == "no provider" => {
                    if let Some(ref timeout) = timeout {
                        if &now.elapsed() > timeout {
                            return Err(ErrorKind::TimeoutError.into());
                        }
                    }
                    sleep(Duration::from_millis(100));
                    continue;
                }
                _ => {}
            }
            return Err(e.into());
        }
    }

    pub fn service<T, F>(&mut self, service: &str, handler: F) -> Result<()>
    where
        T: ServicePair,
        F: Fn(T::Request) -> ServiceResult<T::Response> + Send + Sync + 'static,
    {
        let name = self.resolver.translate(service)?;
        let api = self.slave.add_service::<T, F>(
            &self.hostname,
            &name,
            handler,
        )?;

        if let Err(err) = self.master.register_service(&name, &api) {
            self.slave.remove_service(&name);
            self.master.unregister_service(&name, &api)?;
            Err(err.into())
        } else {
            Ok(())
        }

    }

    pub fn subscribe<T, F>(&mut self, topic: &str, callback: F) -> Result<()>
    where
        T: Message,
        F: Fn(T) -> () + Send + 'static,
    {
        let name = self.resolver.translate(topic)?;
        self.slave.add_subscription::<T, F>(&name, callback)?;

        match self.master.register_subscriber(&name, &T::msg_type()) {
            Ok(publishers) => {
                if let Err(err) = self.slave.add_publishers_to_subscription(
                    &name,
                    publishers.into_iter(),
                )
                {
                    error!(
                        "Failed to subscribe to all publishers of topic '{}': {}",
                        name,
                        err
                    );
                }
                Ok(())
            }
            Err(err) => {
                self.slave.remove_subscription(&name);
                self.master.unregister_subscriber(&name)?;
                Err(err.into())
            }
        }
    }

    pub fn publish<T>(&mut self, topic: &str) -> Result<PublisherStream<T>>
    where
        T: Message,
    {
        let name = self.resolver.translate(topic)?;
        let stream = self.slave.add_publication::<T>(&self.hostname, &name)?;
        match self.master.register_publisher(&name, &T::msg_type()) {
            Ok(_) => Ok(stream),
            Err(error) => {
                error!(
                    "Failed to register publisher for topic '{}': {}",
                    name,
                    error
                );
                self.slave.remove_publication(&name);
                self.master.unregister_publisher(&name)?;
                Err(error.into())
            }
        }
    }
}

pub struct Parameter<'a> {
    master: &'a Master,
    name: String,
}

impl<'a> Parameter<'a> {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn get<'b, T: Deserialize<'b>>(&self) -> Response<T> {
        self.master.get_param::<T>(&self.name)
    }

    pub fn get_raw(&self) -> Response<xml_rpc::Value> {
        self.master.get_param_any(&self.name)
    }

    pub fn set<T: Serialize>(&self, value: &T) -> Response<()> {
        self.master.set_param::<T>(&self.name, value).and(Ok(()))
    }

    pub fn delete(&self) -> Response<()> {
        self.master.delete_param(&self.name).and(Ok(()))
    }

    pub fn exists(&self) -> Response<bool> {
        self.master.has_param(&self.name)
    }

    pub fn search(&self) -> Response<String> {
        self.master.search_param(&self.name)
    }
}
